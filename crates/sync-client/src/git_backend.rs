//! Implementación de `SyncBackend` sobre `git2` (`libgit2` embebido) (`[RF-07]`, `[RF-08]`).
//!
//! Ejecuta commits locales con bloqueo preventivo de archivos > 50 MB, `fetch`,
//! fusión de 3 vías (`merge_trees`) y resolución automática sin pérdida de datos
//! creando `nota (conflicto <dispositivo> <fecha>).md` cuando dos dispositivos
//! editan las mismas líneas de una nota.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use git2::build::CheckoutBuilder;
use git2::{
    AnnotatedCommit, AutotagOption, Cred, FetchOptions, IndexAddOption, IndexEntry, MergeOptions,
    PushOptions, RemoteCallbacks, Repository, RepositoryInitOptions, Signature, StatusOptions,
};
use romsidian_core::{Vault, VAULT_METADATA_DIR};

use crate::credentials::{CredentialStore, OsKeyringStore};
use crate::{
    SyncBackend, SyncConfig, SyncError, SyncReport, SyncResult, SyncState, SyncStatus,
    MAX_SYNC_FILE_SIZE_BYTES,
};

/// Nombre del archivo de configuración de sincronización dentro de `.romsidian/`.
pub const SYNC_CONFIG_FILENAME: &str = "sync.json";

/// Motor de sincronización Git basado en `git2` (`libgit2`).
pub struct GitSyncBackend {
    vault_root: PathBuf,
    config: SyncConfig,
    credentials: Arc<dyn CredentialStore>,
}

impl GitSyncBackend {
    /// Abre o inicializa el repositorio Git en la bóveda dada usando el almacén de credenciales del SO.
    pub fn open_or_init(vault_root: impl AsRef<Path>, config: SyncConfig) -> SyncResult<Self> {
        Self::open_or_init_with_store(vault_root, config, Arc::new(OsKeyringStore::new()))
    }

    /// Abre o inicializa el repositorio Git en la bóveda dada inyectando un `CredentialStore`
    /// personalizado (como `MemoryCredentialStore` en pruebas automatizadas).
    pub fn open_or_init_with_store(
        vault_root: impl AsRef<Path>,
        mut config: SyncConfig,
        credentials: Arc<dyn CredentialStore>,
    ) -> SyncResult<Self> {
        fs::create_dir_all(vault_root.as_ref())?;
        // Asegura que la bóveda tenga `.romsidian/` y `.gitignore` configurado.
        let vault = Vault::open(vault_root.as_ref())
            .map_err(|e| SyncError::Backend(format!("Error al abrir bóveda: {e}")))?;
        let canonical_root = vault.root_path().to_path_buf();

        if config.branch.trim().is_empty() {
            config.branch = "main".to_string();
        }
        if config.device_name.trim().is_empty() {
            config.device_name = "pc-windows".to_string();
        }
        if config.author_name.trim().is_empty() {
            config.author_name = "RomSidian User".to_string();
        }
        if config.author_email.trim().is_empty() {
            config.author_email = "user@romsidian.local".to_string();
        }

        let backend = Self {
            vault_root: canonical_root,
            config,
            credentials,
        };

        backend.ensure_repository()?;
        backend.save_config()?;
        Ok(backend)
    }

    /// Carga la configuración guardada en `.romsidian/sync.json` o devuelve valores por defecto.
    pub fn load_config(vault_root: impl AsRef<Path>) -> SyncResult<SyncConfig> {
        let cfg_path = vault_root
            .as_ref()
            .join(VAULT_METADATA_DIR)
            .join(SYNC_CONFIG_FILENAME);
        if !cfg_path.exists() {
            return Ok(SyncConfig::default());
        }
        let raw = fs::read_to_string(&cfg_path)?;
        // Verificación defensiva: el archivo de configuración nunca debe contener tokens.
        if raw.contains("ghp_") || raw.contains("github_pat_") {
            return Err(SyncError::Credentials(
                "Se detectó un token en texto plano dentro de .romsidian/sync.json; operación bloqueada por seguridad".to_string(),
            ));
        }
        let cfg: SyncConfig = serde_json::from_str(&raw)
            .map_err(|e| SyncError::Backend(format!("Configuración sync.json inválida: {e}")))?;
        Ok(cfg)
    }

    /// Guarda la configuración no sensible en `.romsidian/sync.json`.
    pub fn save_config(&self) -> SyncResult<()> {
        let meta_dir = self.vault_root.join(VAULT_METADATA_DIR);
        fs::create_dir_all(&meta_dir)?;
        let cfg_path = meta_dir.join(SYNC_CONFIG_FILENAME);
        let json = serde_json::to_string_pretty(&self.config)
            .map_err(|e| SyncError::Backend(format!("Error al serializar sync.json: {e}")))?;
        fs::write(cfg_path, json)?;
        Ok(())
    }

    /// Devuelve una referencia a la configuración activa.
    pub fn config(&self) -> &SyncConfig {
        &self.config
    }

    /// Actualiza la configuración de sincronización y reconfigura el remoto `origin`.
    pub fn update_config(&mut self, new_config: SyncConfig) -> SyncResult<()> {
        self.config = new_config;
        self.ensure_repository()?;
        self.save_config()?;
        Ok(())
    }

    /// Guarda el Personal Access Token (PAT) en el `CredentialStore` seguro (nunca en la bóveda).
    pub fn set_pat_token(&self, token: &str) -> SyncResult<()> {
        self.credentials.set_token(&self.account_key(), token)
    }

    /// Verifica si existe un Personal Access Token configurado en el `CredentialStore`.
    pub fn has_pat_token(&self) -> SyncResult<bool> {
        Ok(self.credentials.get_token(&self.account_key())?.is_some())
    }

    /// Elimina el Personal Access Token del `CredentialStore`.
    pub fn clear_pat_token(&self) -> SyncResult<()> {
        self.credentials.delete_token(&self.account_key())
    }

    fn account_key(&self) -> String {
        format!(
            "vault::{}",
            self.vault_root.to_string_lossy().replace('\\', "/")
        )
    }

    fn open_repo(&self) -> SyncResult<Repository> {
        Repository::open(&self.vault_root).map_err(SyncError::from)
    }

    fn ensure_repository(&self) -> SyncResult<Repository> {
        let repo = match Repository::open(&self.vault_root) {
            Ok(r) => r,
            Err(_) => {
                let mut opts = RepositoryInitOptions::new();
                opts.initial_head(&self.config.branch);
                Repository::init_opts(&self.vault_root, &opts)?
            }
        };

        // Desactivamos conversión automática CRLF para preservar bytes exactos entre Windows y Android.
        if let Ok(mut git_cfg) = repo.config() {
            let _ = git_cfg.set_bool("core.autocrlf", false);
        }

        let remote_url = self.config.remote_url.trim();
        if !remote_url.is_empty() {
            if repo.find_remote("origin").is_ok() {
                repo.remote_set_url("origin", remote_url)?;
            } else {
                repo.remote("origin", remote_url)?;
            }
        }

        Ok(repo)
    }

    fn signature(&self) -> SyncResult<Signature<'static>> {
        Signature::now(&self.config.author_name, &self.config.author_email)
            .map_err(SyncError::from)
    }

    fn build_remote_callbacks(&self) -> RemoteCallbacks<'_> {
        let mut callbacks = RemoteCallbacks::new();
        let account_key = self.account_key();
        let store = Arc::clone(&self.credentials);

        callbacks.credentials(move |_url, username_from_url, _allowed_types| {
            if let Ok(Some(token)) = store.get_token(&account_key) {
                let user = username_from_url.unwrap_or("x-access-token");
                return Cred::userpass_plaintext(user, &token);
            }
            Cred::default()
        });

        callbacks
    }

    /// Escanea todos los archivos de la bóveda (excluyendo `.git/` y `.romsidian/`)
    /// y bloquea preventivamente cualquier archivo que supere `MAX_SYNC_FILE_SIZE_BYTES` (50 MB).
    pub fn enforce_file_size_limit(&self) -> SyncResult<()> {
        check_dir_file_sizes(&self.vault_root, &self.vault_root)
    }
}

impl SyncBackend for GitSyncBackend {
    fn status(&self) -> SyncResult<SyncStatus> {
        let repo = self.open_repo()?;
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .exclude_submodules(true);

        let statuses = repo.statuses(Some(&mut opts))?;
        let mut modified_files = Vec::new();
        let mut conflict_files = Vec::new();

        for entry in statuses.iter() {
            let Some(path) = entry.path() else {
                continue;
            };
            let norm = path.replace('\\', "/");
            if norm == VAULT_METADATA_DIR || norm.starts_with(&format!("{VAULT_METADATA_DIR}/")) {
                continue;
            }
            if entry.status().is_conflicted() || is_conflict_copy_filename(&norm) {
                conflict_files.push(norm.clone());
            }
            if !entry.status().is_ignored() {
                modified_files.push(norm);
            }
        }

        // También detectamos copias de conflicto ya confirmadas en disco para informar en la UI.
        collect_conflict_copy_files(&self.vault_root, &self.vault_root, &mut conflict_files);
        conflict_files.sort();
        conflict_files.dedup();
        modified_files.sort();
        modified_files.dedup();

        let last_commit_id = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .map(|oid| oid.to_string());

        let state = if !conflict_files.is_empty() {
            SyncState::ConflictDetected {
                conflict_files: conflict_files.clone(),
            }
        } else {
            SyncState::UpToDate
        };

        let remote_url = if self.config.remote_url.trim().is_empty() {
            None
        } else {
            Some(self.config.remote_url.clone())
        };

        Ok(SyncStatus {
            state,
            branch: self.config.branch.clone(),
            remote_url,
            modified_files,
            conflict_files,
            last_commit_id,
        })
    }

    fn commit_local(&mut self, label: Option<&str>) -> SyncResult<Option<String>> {
        // 1. Bloqueo preventivo de archivos > 50 MB ([RF-08])
        self.enforce_file_size_limit()?;

        let repo = self.ensure_repository()?;
        let mut index = repo.index()?;

        // 2. Añadir cambios del árbol de trabajo y eliminar borrados, excluyendo `.romsidian/`
        index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
        index.update_all(["*"].iter(), None)?;
        let _ = index.remove_dir(Path::new(VAULT_METADATA_DIR), 0);
        index.write()?;

        let tree_oid = index.write_tree()?;
        let tree = repo.find_tree(tree_oid)?;

        let parent_commit = match repo.head() {
            Ok(head_ref) => Some(head_ref.peel_to_commit()?),
            Err(_) => None,
        };

        if let Some(ref parent) = parent_commit {
            if parent.tree()?.id() == tree_oid {
                return Ok(None);
            }
        } else if index.is_empty() {
            return Ok(None);
        }

        let ts = label.unwrap_or("auto");
        let commit_msg = format!("sync({}): {}", self.config.device_name, ts);
        let sig = self.signature()?;

        let commit_oid = if let Some(ref parent) = parent_commit {
            repo.commit(Some("HEAD"), &sig, &sig, &commit_msg, &tree, &[parent])?
        } else {
            let branch_ref = format!("refs/heads/{}", self.config.branch);
            let oid = repo.commit(Some(&branch_ref), &sig, &sig, &commit_msg, &tree, &[])?;
            let _ = repo.set_head(&branch_ref);
            oid
        };

        Ok(Some(commit_oid.to_string()))
    }

    fn pull_and_merge(&mut self, date_label: &str) -> SyncResult<SyncReport> {
        if self.config.remote_url.trim().is_empty() {
            return Err(SyncError::MissingRemoteUrl);
        }

        let repo = self.ensure_repository()?;
        let mut remote = repo
            .find_remote("origin")
            .map_err(|e| SyncError::Network(format!("Remoto 'origin' inválido: {e}")))?;

        let mut fetch_opts = FetchOptions::new();
        fetch_opts.remote_callbacks(self.build_remote_callbacks());
        fetch_opts.download_tags(AutotagOption::None);

        let refspec = format!(
            "+refs/heads/{0}:refs/remotes/origin/{0}",
            self.config.branch
        );
        remote
            .fetch(&[&refspec], Some(&mut fetch_opts), None)
            .map_err(|e| {
                SyncError::Network(format!(
                    "Fallo de red o acceso al ejecutar fetch desde '{}': {}",
                    self.config.remote_url,
                    e.message()
                ))
            })?;

        let remote_ref_name = format!("refs/remotes/origin/{}", self.config.branch);
        let Ok(remote_ref) = repo.find_reference(&remote_ref_name) else {
            // El repositorio remoto aún no tiene la rama publicada (repositorio vacío).
            let head_commit = repo
                .head()
                .ok()
                .and_then(|h| h.target())
                .map(|o| o.to_string());
            return Ok(SyncReport {
                state: SyncState::UpToDate,
                committed_files: 0,
                merged_cleanly: Vec::new(),
                conflict_copies_created: Vec::new(),
                pushed: false,
                head_commit,
            });
        };

        let remote_oid = remote_ref
            .target()
            .ok_or_else(|| SyncError::Backend("Referencia remota sin OID directo".to_string()))?;
        let remote_commit = repo.find_commit(remote_oid)?;
        let annotated_remote = repo.find_annotated_commit(remote_oid)?;

        // Si HEAD local aún no tiene commits, hacemos fast-forward inicial hacia origin/<branch>
        let local_commit = match repo.head() {
            Ok(head) => head.peel_to_commit()?,
            Err(_) => {
                self.fast_forward_to(&repo, &remote_commit)?;
                return Ok(SyncReport {
                    state: SyncState::UpToDate,
                    committed_files: 0,
                    merged_cleanly: Vec::new(),
                    conflict_copies_created: Vec::new(),
                    pushed: false,
                    head_commit: Some(remote_oid.to_string()),
                });
            }
        };

        if local_commit.id() == remote_commit.id() {
            return Ok(SyncReport {
                state: SyncState::UpToDate,
                committed_files: 0,
                merged_cleanly: Vec::new(),
                conflict_copies_created: Vec::new(),
                pushed: false,
                head_commit: Some(local_commit.id().to_string()),
            });
        }

        let (analysis, _) = repo.merge_analysis(&[&annotated_remote])?;

        if analysis.is_up_to_date() {
            return Ok(SyncReport {
                state: SyncState::UpToDate,
                committed_files: 0,
                merged_cleanly: Vec::new(),
                conflict_copies_created: Vec::new(),
                pushed: false,
                head_commit: Some(local_commit.id().to_string()),
            });
        }

        if analysis.is_fast_forward() {
            let changed = diff_tree_paths(&repo, &local_commit.tree()?, &remote_commit.tree()?)?;
            self.fast_forward_to(&repo, &remote_commit)?;
            return Ok(SyncReport {
                state: SyncState::UpToDate,
                committed_files: 0,
                merged_cleanly: changed,
                conflict_copies_created: Vec::new(),
                pushed: false,
                head_commit: Some(remote_commit.id().to_string()),
            });
        }

        // Historiales divergentes: ejecutamos fusión de 3 vías (diff3) en memoria
        self.merge_three_way(&repo, &local_commit, &remote_commit, &annotated_remote, date_label)
    }

    fn push(&mut self) -> SyncResult<()> {
        if self.config.remote_url.trim().is_empty() {
            return Err(SyncError::MissingRemoteUrl);
        }

        let repo = self.ensure_repository()?;
        if repo.head().is_err() {
            return Ok(());
        }

        let mut remote = repo
            .find_remote("origin")
            .map_err(|e| SyncError::Network(format!("Remoto 'origin' inválido: {e}")))?;

        let push_error: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));
        let push_err_clone = Arc::clone(&push_error);

        let mut callbacks = self.build_remote_callbacks();
        callbacks.push_update_reference(move |refname, status| {
            if let Some(msg) = status {
                if let Ok(mut guard) = push_err_clone.lock() {
                    *guard = Some(format!("Rechazo en referencia {refname}: {msg}"));
                }
            }
            Ok(())
        });

        let mut push_opts = PushOptions::new();
        push_opts.remote_callbacks(callbacks);

        let refspec = format!(
            "refs/heads/{0}:refs/heads/{0}",
            self.config.branch
        );
        remote
            .push(&[&refspec], Some(&mut push_opts))
            .map_err(|e| {
                SyncError::Network(format!(
                    "Fallo de red o acceso al ejecutar push hacia '{}': {}",
                    self.config.remote_url,
                    e.message()
                ))
            })?;

        if let Ok(guard) = push_error.lock() {
            if let Some(ref err_msg) = *guard {
                return Err(SyncError::Network(err_msg.clone()));
            }
        }

        Ok(())
    }

    fn sync_now(&mut self, timestamp_label: &str) -> SyncResult<SyncReport> {
        #[cfg(target_os = "android")]
        {
            if self.config.remote_url.trim().is_empty() {
                return Err(SyncError::MissingRemoteUrl);
            }
            let _ = self.ensure_repository()?;
            let token = self.credentials.get_token(&self.account_key())?;
            return crate::android_github::sync_github_android(
                &self.vault_root,
                &self.config,
                token,
                timestamp_label,
            );
        }
        #[cfg(not(target_os = "android"))]
        {
            let local_commit = self.commit_local(Some(timestamp_label))?;
            let committed_files = usize::from(local_commit.is_some());

            let date_only = timestamp_label
                .split_whitespace()
                .next()
                .unwrap_or(timestamp_label);

            let mut report = self.pull_and_merge(date_only)?;
            report.committed_files = committed_files;

            self.push()?;
            report.pushed = true;
            report.head_commit = self
                .open_repo()
                .ok()
                .and_then(|r| r.head().ok()?.target())
                .map(|oid| oid.to_string());

            Ok(report)
        }
    }
}

impl GitSyncBackend {
    fn fast_forward_to(
        &self,
        repo: &Repository,
        target_commit: &git2::Commit<'_>,
    ) -> SyncResult<()> {
        let target_tree = target_commit.tree()?;
        repo.checkout_tree(
            target_tree.as_object(),
            Some(CheckoutBuilder::new().force()),
        )?;

        let branch_ref = format!("refs/heads/{}", self.config.branch);
        repo.reference(
            &branch_ref,
            target_commit.id(),
            true,
            "sync: fast-forward",
        )?;
        repo.set_head(&branch_ref)?;
        Ok(())
    }

    fn merge_three_way(
        &self,
        repo: &Repository,
        local_commit: &git2::Commit<'_>,
        remote_commit: &git2::Commit<'_>,
        _annotated_remote: &AnnotatedCommit<'_>,
        date_label: &str,
    ) -> SyncResult<SyncReport> {
        let ancestor_tree = match repo.merge_base(local_commit.id(), remote_commit.id()) {
            Ok(base_oid) => repo.find_commit(base_oid)?.tree()?,
            Err(_) => {
                let empty_oid = repo.treebuilder(None)?.write()?;
                repo.find_tree(empty_oid)?
            }
        };

        let local_tree = local_commit.tree()?;
        let remote_tree = remote_commit.tree()?;

        let mut merge_opts = MergeOptions::new();
        let mut merged_index =
            repo.merge_trees(&ancestor_tree, &local_tree, &remote_tree, Some(&mut merge_opts))?;

        let mut conflict_copies_created = Vec::new();
        let mut local_copies_to_write: Vec<(String, Vec<u8>)> = Vec::new();

        if merged_index.has_conflicts() {
            // Recolectamos todos los conflictos antes de mutar merged_index
            let mut raw_conflicts: Vec<(Option<IndexEntry>, Option<IndexEntry>, Option<IndexEntry>)> =
                Vec::new();
            {
                let conflicts_iter = merged_index.conflicts()?;
                for conflict in conflicts_iter {
                    let c = conflict?;
                    raw_conflicts.push((c.ancestor, c.our, c.their));
                }
            }

            for (ancestor, our, their) in raw_conflicts {
                let rel_path = their
                    .as_ref()
                    .or(our.as_ref())
                    .or(ancestor.as_ref())
                    .and_then(|e| std::str::from_utf8(&e.path).ok())
                    .ok_or_else(|| {
                        SyncError::Backend("Ruta de archivo en conflicto no es UTF-8 válido".to_string())
                    })?
                    .to_string();

                // 1. Si existe versión local (`our`), la preservamos íntegra en `nota (conflicto <dispositivo> <fecha>).md`
                if let Some(ref our_entry) = our {
                    let our_blob = repo.find_blob(our_entry.id)?;
                    let conflict_rel_path = build_conflict_copy_path(
                        &self.vault_root,
                        &rel_path,
                        &self.config.device_name,
                        date_label,
                    );
                    local_copies_to_write
                        .push((conflict_rel_path.clone(), our_blob.content().to_vec()));
                    conflict_copies_created.push(conflict_rel_path);
                }

                // 2. Limpiamos las etapas de conflicto en merged_index y dejamos la versión remota (`their`)
                // en `nota.md` (tal como exige [RF-07]).
                merged_index.conflict_remove(Path::new(&rel_path))?;
                if let Some(mut their_entry) = their {
                    // Limpiamos los bits de stage (bits 12..13 de flags) para dejar stage 0
                    their_entry.flags &= !0x3000;
                    merged_index.add(&their_entry)?;
                }
            }
        }

        // Ahora merged_index no tiene conflictos: escribimos su árbol base y lo volcamos a disco
        let base_merged_tree_oid = merged_index.write_tree_to(repo)?;
        let base_merged_tree = repo.find_tree(base_merged_tree_oid)?;
        repo.checkout_tree(
            base_merged_tree.as_object(),
            Some(CheckoutBuilder::new().force()),
        )?;

        // Escribimos en disco cada archivo `nota (conflicto <dispositivo> <fecha>).md`
        for (conflict_rel_path, bytes) in &local_copies_to_write {
            let full_conflict_path = self.vault_root.join(conflict_rel_path);
            if let Some(parent) = full_conflict_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full_conflict_path, bytes)?;
        }

        // Sincronizamos el índice principal del repositorio con el árbol fusionado + copias de conflicto
        let mut repo_index = repo.index()?;
        repo_index.read_tree(&base_merged_tree)?;
        for (conflict_rel_path, _) in &local_copies_to_write {
            repo_index.add_path(Path::new(conflict_rel_path))?;
        }
        let _ = repo_index.remove_dir(Path::new(VAULT_METADATA_DIR), 0);
        repo_index.write()?;

        let final_tree_oid = repo_index.write_tree()?;
        let final_tree = repo.find_tree(final_tree_oid)?;

        let merged_cleanly = diff_tree_paths(repo, &local_tree, &final_tree)?;

        let sig = self.signature()?;
        let merge_msg = if conflict_copies_created.is_empty() {
            format!(
                "sync({}): merge origin/{}",
                self.config.device_name, self.config.branch
            )
        } else {
            format!(
                "sync({}): merge origin/{} (conflictos preservados: {})",
                self.config.device_name,
                self.config.branch,
                conflict_copies_created.join(", ")
            )
        };

        let merge_commit_oid = repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &merge_msg,
            &final_tree,
            &[local_commit, remote_commit],
        )?;

        let state = if conflict_copies_created.is_empty() {
            SyncState::UpToDate
        } else {
            SyncState::ConflictDetected {
                conflict_files: conflict_copies_created.clone(),
            }
        };

        Ok(SyncReport {
            state,
            committed_files: 0,
            merged_cleanly,
            conflict_copies_created,
            pushed: false,
            head_commit: Some(merge_commit_oid.to_string()),
        })
    }
}

/// Construye la ruta relativa `nota (conflicto <dispositivo> <fecha>).md` respetando subcarpetas.
///
/// Si ya existe un archivo con ese mismo nombre en disco, añade un sufijo incremental (`-2`, `-3`...)
/// para garantizar cero pérdida de datos incluso ante múltiples conflictos el mismo día.
pub fn build_conflict_copy_path(
    vault_root: &Path,
    original_rel_path: &str,
    device_name: &str,
    date_label: &str,
) -> String {
    let norm = original_rel_path.replace('\\', "/");
    let norm = norm.trim_start_matches('/');
    let path = Path::new(norm);

    let parent_prefix = path
        .parent()
        .and_then(|p| p.to_str())
        .map(|s| s.replace('\\', "/"))
        .unwrap_or_default();

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("nota");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("md");

    let safe_device: String = device_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let safe_date: String = date_label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();

    let base_candidate = if parent_prefix.is_empty() {
        format!("{stem} (conflicto {safe_device} {safe_date}).{ext}")
    } else {
        format!("{parent_prefix}/{stem} (conflicto {safe_device} {safe_date}).{ext}")
    };

    if !vault_root.join(&base_candidate).exists() {
        return base_candidate;
    }

    for counter in 2..1000 {
        let candidate = if parent_prefix.is_empty() {
            format!("{stem} (conflicto {safe_device} {safe_date}-{counter}).{ext}")
        } else {
            format!("{parent_prefix}/{stem} (conflicto {safe_device} {safe_date}-{counter}).{ext}")
        };
        if !vault_root.join(&candidate).exists() {
            return candidate;
        }
    }

    base_candidate
}

fn is_conflict_copy_filename(rel_path: &str) -> bool {
    rel_path.contains(" (conflicto ") && rel_path.ends_with(".md")
}

fn collect_conflict_copy_files(root: &Path, current: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_conflict_copy_files(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            let norm = rel.to_string_lossy().replace('\\', "/");
            if is_conflict_copy_filename(&norm) {
                out.push(norm);
            }
        }
    }
}

fn check_dir_file_sizes(root: &Path, current: &Path) -> SyncResult<()> {
    let entries = fs::read_dir(current)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Ignoramos carpetas ocultas (.git, .romsidian)
        if name_str.starts_with('.') {
            continue;
        }

        let meta = entry.metadata()?;
        if meta.is_dir() {
            check_dir_file_sizes(root, &path)?;
        } else if meta.len() > MAX_SYNC_FILE_SIZE_BYTES {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            return Err(SyncError::FileTooLarge {
                path: rel,
                size_bytes: meta.len(),
            });
        }
    }
    Ok(())
}

fn diff_tree_paths(
    repo: &Repository,
    old_tree: &git2::Tree<'_>,
    new_tree: &git2::Tree<'_>,
) -> SyncResult<Vec<String>> {
    let diff = repo.diff_tree_to_tree(Some(old_tree), Some(new_tree), None)?;
    let mut paths = Vec::new();
    for delta in diff.deltas() {
        if let Some(p) = delta.new_file().path().or_else(|| delta.old_file().path()) {
            paths.push(p.to_string_lossy().replace('\\', "/"));
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
