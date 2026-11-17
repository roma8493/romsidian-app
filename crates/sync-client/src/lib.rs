//! Motor de sincronización de ROMSIDIAN (`romsidian-sync`).
//!
//! Define el contrato `SyncBackend`, la gestión segura de credenciales (`CredentialStore`)
//! y el backend Git (`GitSyncBackend` sobre `git2`) con fusión de 3 vías y copias en conflicto
//! (`nota (conflicto <dispositivo> <fecha>).md`) garantizando cero pérdida de datos.

pub mod credentials;
pub mod git_backend;
#[cfg(target_os = "android")]
pub mod android_github;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use credentials::{
    set_android_secrets_dir, CredentialStore, MemoryCredentialStore, OsKeyringStore,
    KEYRING_SERVICE_NAME,
};
pub use git_backend::{build_conflict_copy_path, GitSyncBackend, SYNC_CONFIG_FILENAME};

/// Límite máximo por archivo individual permitido antes de sincronizar con GitHub (50 MB).
pub const MAX_SYNC_FILE_SIZE_BYTES: u64 = 50 * 1024 * 1024;

/// Errores del cliente de sincronización.
#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Error de entrada/salida: {0}")]
    Io(#[from] std::io::Error),

    #[error("Error en el repositorio Git local: {0}")]
    Git(#[from] git2::Error),

    #[error("Archivo excede el límite de 50 MB ({size_bytes} bytes): {path}")]
    FileTooLarge { path: String, size_bytes: u64 },

    #[error("No se ha configurado la URL del repositorio remoto (origin)")]
    MissingRemoteUrl,

    #[error("Error de red o acceso al repositorio remoto: {0}")]
    Network(String),

    #[error("Error en el almacén seguro de credenciales: {0}")]
    Credentials(String),

    #[error("Error de sincronización: {0}")]
    Backend(String),
}

/// Resultado estándar para operaciones de `romsidian-sync`.
pub type SyncResult<T> = Result<T, SyncError>;

/// Estado resumido de sincronización de una bóveda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncState {
    Idle,
    Syncing,
    UpToDate,
    ConflictDetected { conflict_files: Vec<String> },
    Error { message: String },
}

/// Configuración persistible de sincronización de una bóveda (guardada en `.romsidian/sync.json`).
///
/// **Importante**: Nunca almacena tokens ni contraseñas; estos residen en `CredentialStore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConfig {
    /// URL del repositorio remoto (ej. `https://github.com/usuario/boveda.git` o ruta local en tests).
    pub remote_url: String,
    /// Rama principal de sincronización (por defecto `"main"`).
    pub branch: String,
    /// Identificador legible del dispositivo actual (ej. `"pc-windows"`, `"android"`).
    pub device_name: String,
    /// Nombre del autor para los commits Git.
    pub author_name: String,
    /// Correo del autor para los commits Git.
    pub author_email: String,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            remote_url: String::new(),
            branch: "main".to_string(),
            device_name: if cfg!(target_os = "android") {
                "android".to_string()
            } else {
                "pc-windows".to_string()
            },
            author_name: "RomSidian User".to_string(),
            author_email: "user@romsidian.local".to_string(),
        }
    }
}

/// Estado detallado de sincronización de la bóveda para mostrar en la interfaz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    pub state: SyncState,
    pub branch: String,
    pub remote_url: Option<String>,
    pub modified_files: Vec<String>,
    pub conflict_files: Vec<String>,
    pub last_commit_id: Option<String>,
}

/// Informe devuelto al finalizar una operación de sincronización (`pull_and_merge` o `sync_now`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    pub state: SyncState,
    pub committed_files: usize,
    pub merged_cleanly: Vec<String>,
    pub conflict_copies_created: Vec<String>,
    pub pushed: bool,
    pub head_commit: Option<String>,
}

/// Interfaz intercambiable para el motor de sincronización de ROMSIDIAN (`[RF-07]`).
pub trait SyncBackend {
    /// Inspecciona el estado actual de la bóveda (archivos modificados, conflictos y último commit).
    fn status(&self) -> SyncResult<SyncStatus>;

    /// Valida el límite de 50 MB por archivo y crea un commit local si hay cambios en la bóveda.
    fn commit_local(&mut self, label: Option<&str>) -> SyncResult<Option<String>>;

    /// Descarga (`fetch`) desde el remoto y ejecuta fusión de 3 vías creando copias en conflicto si hay colisión.
    fn pull_and_merge(&mut self, date_label: &str) -> SyncResult<SyncReport>;

    /// Envía (`push`) los commits locales hacia el repositorio remoto.
    fn push(&mut self) -> SyncResult<()>;

    /// Ejecuta el flujo completo de sincronización: `commit_local` -> `pull_and_merge` -> `push`.
    fn sync_now(&mut self, timestamp_label: &str) -> SyncResult<SyncReport>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, RepositoryInitOptions};
    use romsidian_core::VAULT_METADATA_DIR;
    use std::env;
    use std::fs::{self, File};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Reloj del sistema válido")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "romsidian_sync_test_{}_{}_{}",
            prefix,
            std::process::id(),
            unique
        ));
        fs::create_dir_all(&dir).expect("Crear directorio temporal");
        dir
    }

    fn init_bare_remote(path: &Path) {
        let mut opts = RepositoryInitOptions::new();
        opts.bare(true);
        opts.initial_head("main");
        Repository::init_opts(path, &opts).expect("Inicializar repositorio bare remoto");
    }

    #[test]
    fn max_file_size_limit_is_50_megabytes_and_blocks_oversized_file() {
        assert_eq!(MAX_SYNC_FILE_SIZE_BYTES, 52_428_800);

        let temp_dir = create_temp_dir("oversized_guard");
        let cred_store = Arc::new(MemoryCredentialStore::new());
        let mut backend = GitSyncBackend::open_or_init_with_store(
            &temp_dir,
            SyncConfig::default(),
            cred_store,
        )
        .expect("Abrir backend Git");

        // Creamos un archivo disperso (sparse) de 50 MB + 1 byte sin consumir RAM
        let big_file_path = temp_dir.join("archivo_pesado.bin");
        let file = File::create(&big_file_path).expect("Crear archivo grande");
        file.set_len(MAX_SYNC_FILE_SIZE_BYTES + 1)
            .expect("Asignar longitud > 50 MB");

        let err = backend
            .commit_local(Some("2026-11-11 23:00"))
            .expect_err("Debe bloquear el commit de un archivo > 50 MB");

        match err {
            SyncError::FileTooLarge { path, size_bytes } => {
                assert_eq!(path, "archivo_pesado.bin");
                assert_eq!(size_bytes, MAX_SYNC_FILE_SIZE_BYTES + 1);
            }
            other => panic!("Se esperaba FileTooLarge, se obtuvo: {other:?}"),
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn credential_store_saves_token_outside_vault_and_blocks_plaintext_tokens() {
        let temp_dir = create_temp_dir("creds_security");
        let cred_store = Arc::new(MemoryCredentialStore::new());
        let cfg = SyncConfig {
            remote_url: "https://github.com/usuario/boveda-privada.git".to_string(),
            branch: "main".to_string(),
            device_name: if cfg!(target_os = "android") {
                "android".to_string()
            } else {
                "pc-windows".to_string()
            },
            ..Default::default()
        };

        let backend =
            GitSyncBackend::open_or_init_with_store(&temp_dir, cfg, cred_store.clone())
                .expect("Inicializar backend");

        assert!(!backend.has_pat_token().unwrap());
        backend
            .set_pat_token("github_pat_11AABBCCDD_token_secreto")
            .expect("Guardar token en almacén seguro");
        assert!(backend.has_pat_token().unwrap());

        // Verificamos que `.romsidian/sync.json` no contenga el token bajo ninguna circunstancia
        let sync_json_path = temp_dir
            .join(VAULT_METADATA_DIR)
            .join(SYNC_CONFIG_FILENAME);
        let raw_json = fs::read_to_string(&sync_json_path).expect("Leer sync.json");
        assert!(
            !raw_json.contains("github_pat_"),
            "sync.json jamás debe contener el token PAT"
        );

        // Si alguien inyecta manualmente un token en sync.json, load_config debe bloquearlo
        fs::write(
            &sync_json_path,
            r#"{"remote_url":"https://ghp_secret12345@github.com/u/r.git","branch":"main","device_name":"pc","author_name":"A","author_email":"a@b.c"}"#,
        )
        .unwrap();
        let load_err = GitSyncBackend::load_config(&temp_dir)
            .expect_err("Debe rechazar sync.json contaminado con token");
        assert!(matches!(load_err, SyncError::Credentials(_)));

        backend.clear_pat_token().expect("Limpiar token");
        assert!(!backend.has_pat_token().unwrap());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn two_clones_clean_three_way_merge_on_different_lines_and_files() {
        let base_dir = create_temp_dir("clean_3way_merge");
        let remote_dir = base_dir.join("origin.git");
        let pc_dir = base_dir.join("vault_pc");
        let android_dir = base_dir.join("vault_android");

        init_bare_remote(&remote_dir);
        let remote_url = remote_dir.to_string_lossy().to_string();
        let creds = Arc::new(MemoryCredentialStore::new());

        let mut pc_sync = GitSyncBackend::open_or_init_with_store(
            &pc_dir,
            SyncConfig {
                remote_url: remote_url.clone(),
                branch: "main".to_string(),
                device_name: if cfg!(target_os = "android") {
                "android".to_string()
            } else {
                "pc-windows".to_string()
            },
                author_name: "Roma PC".to_string(),
                author_email: "pc@romsidian.local".to_string(),
            },
            creds.clone(),
        )
        .expect("Abrir PC vault");

        // 1. PC crea una nota base con varias líneas y sincroniza con origin.git
        let initial_note = "# Hoja de Ruta\n\nLínea 1: Objetivo inicial\nLínea 2: Separador estable\nLínea 3: Estado estable\nLínea 4: Conclusión original\n";
        fs::write(pc_dir.join("Hoja de Ruta.md"), initial_note).unwrap();
        let rep1 = pc_sync
            .sync_now("2026-11-11 10:00")
            .expect("Primer sync desde PC");
        assert_eq!(rep1.state, SyncState::UpToDate);
        assert!(rep1.pushed);

        // 2. Android se inicializa y descarga el estado inicial desde origin.git
        let mut android_sync = GitSyncBackend::open_or_init_with_store(
            &android_dir,
            SyncConfig {
                remote_url: remote_url.clone(),
                branch: "main".to_string(),
                device_name: "android".to_string(),
                author_name: "Roma Android".to_string(),
                author_email: "android@romsidian.local".to_string(),
            },
            creds.clone(),
        )
        .expect("Abrir Android vault");

        let rep_init_android = android_sync
            .sync_now("2026-11-11 10:05")
            .expect("Pull inicial en Android");
        assert_eq!(rep_init_android.state, SyncState::UpToDate);
        assert!(android_dir.join("Hoja de Ruta.md").exists());

        // 3. Edición concurrente en líneas DISTINTAS del mismo archivo:
        // - PC modifica la Línea 1 y crea además "Nota PC.md"
        let pc_edited = "# Hoja de Ruta\n\nLínea 1: Objetivo actualizado en PC\nLínea 2: Separador estable\nLínea 3: Estado estable\nLínea 4: Conclusión original\n";
        fs::write(pc_dir.join("Hoja de Ruta.md"), pc_edited).unwrap();
        fs::write(pc_dir.join("Nota PC.md"), "# Creada en PC\n").unwrap();
        pc_sync
            .sync_now("2026-11-11 11:00")
            .expect("PC sube sus cambios primero");

        // - Android (sin haber hecho pull aún) modifica la Línea 4 y crea "Nota Android.md"
        let android_edited = "# Hoja de Ruta\n\nLínea 1: Objetivo inicial\nLínea 2: Separador estable\nLínea 3: Estado estable\nLínea 4: Conclusión ampliada desde Android\n";
        fs::write(android_dir.join("Hoja de Ruta.md"), android_edited).unwrap();
        fs::write(android_dir.join("Nota Android.md"), "# Creada en Android\n").unwrap();

        // 4. Android ejecuta sync_now: debe fusionar limpiamente en 3 vías sin crear archivo de conflicto
        let rep_merge = android_sync
            .sync_now("2026-11-11 11:05")
            .expect("Android fusiona en 3 vías y hace push");
        assert_eq!(rep_merge.state, SyncState::UpToDate);
        assert!(
            rep_merge.conflict_copies_created.is_empty(),
            "No debe crearse copia de conflicto cuando se editan líneas distintas"
        );

        let merged_content =
            fs::read_to_string(android_dir.join("Hoja de Ruta.md")).expect("Leer nota fusionada");
        assert!(
            merged_content.contains("Línea 1: Objetivo actualizado en PC"),
            "Debe conservar el cambio de PC en la Línea 1"
        );
        assert!(
            merged_content.contains("Línea 4: Conclusión ampliada desde Android"),
            "Debe conservar el cambio de Android en la Línea 4"
        );
        assert!(android_dir.join("Nota PC.md").exists());
        assert!(android_dir.join("Nota Android.md").exists());

        // 5. PC sincroniza y recibe la fusión limpia y "Nota Android.md"
        pc_sync
            .sync_now("2026-11-11 11:10")
            .expect("PC recibe fusión limpia");
        assert!(pc_dir.join("Nota Android.md").exists());
        assert_eq!(
            fs::read_to_string(pc_dir.join("Hoja de Ruta.md")).unwrap(),
            merged_content
        );

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn two_clones_collision_on_same_line_creates_conflict_copy_with_zero_data_loss() {
        let base_dir = create_temp_dir("conflict_same_line");
        let remote_dir = base_dir.join("origin.git");
        let pc_dir = base_dir.join("vault_pc");
        let android_dir = base_dir.join("vault_android");

        init_bare_remote(&remote_dir);
        let remote_url = remote_dir.to_string_lossy().to_string();
        let creds = Arc::new(MemoryCredentialStore::new());

        let mut pc_sync = GitSyncBackend::open_or_init_with_store(
            &pc_dir,
            SyncConfig {
                remote_url: remote_url.clone(),
                branch: "main".to_string(),
                device_name: if cfg!(target_os = "android") {
                "android".to_string()
            } else {
                "pc-windows".to_string()
            },
                author_name: "Roma PC".to_string(),
                author_email: "pc@romsidian.local".to_string(),
            },
            creds.clone(),
        )
        .expect("Abrir PC vault");

        // 1. Nota inicial con tildes y eñes en PC -> sync a origin
        let base_text = "# Análisis Macroeconómico\n\nProyección de inflación anual: 25%\n";
        fs::write(pc_dir.join("Análisis Macroeconómico.md"), base_text).unwrap();
        pc_sync
            .sync_now("2026-11-11 09:00")
            .expect("Sync base desde PC");

        // 2. Android descarga la nota base
        let mut android_sync = GitSyncBackend::open_or_init_with_store(
            &android_dir,
            SyncConfig {
                remote_url: remote_url.clone(),
                branch: "main".to_string(),
                device_name: "android".to_string(),
                author_name: "Roma Android".to_string(),
                author_email: "android@romsidian.local".to_string(),
            },
            creds.clone(),
        )
        .expect("Abrir Android vault");
        android_sync
            .sync_now("2026-11-11 09:05")
            .expect("Pull base en Android");

        // 3. Ambos dispositivos modifican LA MISMA LÍNEA de "Análisis Macroeconómico.md" sin sincronizar entre sí:
        let pc_version =
            "# Análisis Macroeconómico\n\nProyección de inflación anual: 18% (escenario PC)\n";
        let android_version =
            "# Análisis Macroeconómico\n\nProyección de inflación anual: 32% (escenario móvil Android)\n";

        fs::write(pc_dir.join("Análisis Macroeconómico.md"), pc_version).unwrap();
        pc_sync
            .sync_now("2026-11-11 12:00")
            .expect("PC sube su versión primero");

        fs::write(
            android_dir.join("Análisis Macroeconómico.md"),
            android_version,
        )
        .unwrap();

        // 4. Android sincroniza y detecta colisión en la misma línea:
        // Regla [RF-07]: conservar versión remota en `nota.md` y guardar versión local íntegra en
        // `nota (conflicto <dispositivo> <fecha>).md`.
        let conflict_report = android_sync
            .sync_now("2026-11-11 12:05")
            .expect("Android resuelve conflicto sin pérdida de datos");

        let expected_conflict_filename =
            "Análisis Macroeconómico (conflicto android 2026-11-11).md";
        assert_eq!(
            conflict_report.conflict_copies_created,
            vec![expected_conflict_filename.to_string()]
        );
        assert_eq!(
            conflict_report.state,
            SyncState::ConflictDetected {
                conflict_files: vec![expected_conflict_filename.to_string()]
            }
        );

        let main_note_on_android =
            fs::read_to_string(android_dir.join("Análisis Macroeconómico.md"))
                .expect("Leer nota principal en Android");
        let conflict_copy_on_android =
            fs::read_to_string(android_dir.join(expected_conflict_filename))
                .expect("Leer copia de conflicto en Android");

        assert_eq!(
            main_note_on_android, pc_version,
            "El archivo original nota.md debe conservar la versión remota íntegra sin marcadores <<<<<<<"
        );
        assert_eq!(
            conflict_copy_on_android, android_version,
            "La copia en conflicto debe conservar la versión local íntegra de Android"
        );
        assert!(!main_note_on_android.contains("<<<<<<<"));
        assert!(!conflict_copy_on_android.contains("<<<<<<<"));

        // 5. Cuando PC vuelve a sincronizar, también recibe la copia en conflicto creada por Android
        pc_sync
            .sync_now("2026-11-11 12:10")
            .expect("PC descarga el estado resuelto");
        assert!(pc_dir.join(expected_conflict_filename).exists());
        assert_eq!(
            fs::read_to_string(pc_dir.join(expected_conflict_filename)).unwrap(),
            android_version
        );

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn network_or_invalid_remote_aborts_cleanly_preserving_local_commit() {
        let temp_dir = create_temp_dir("offline_resilience");
        let creds = Arc::new(MemoryCredentialStore::new());
        let mut backend = GitSyncBackend::open_or_init_with_store(
            &temp_dir,
            SyncConfig {
                remote_url: temp_dir
                    .join("ruta_remota_inexistente_404.git")
                    .to_string_lossy()
                    .to_string(),
                branch: "main".to_string(),
                device_name: if cfg!(target_os = "android") {
                "android".to_string()
            } else {
                "pc-windows".to_string()
            },
                ..Default::default()
            },
            creds,
        )
        .expect("Abrir bóveda local");

        fs::write(
            temp_dir.join("Nota Offline.md"),
            "# Guardada sin conexión\n",
        )
        .unwrap();

        let err = backend
            .sync_now("2026-11-11 14:00")
            .expect_err("Debe fallar limpiamente ante remoto inalcanzable");
        assert!(
            matches!(err, SyncError::Network(_)),
            "El error debe ser SyncError::Network, fue: {err:?}"
        );

        // Verificamos que el commit local se haya preservado intacto en HEAD
        let status = backend.status().expect("Consultar estado tras fallo de red");
        assert!(
            status.last_commit_id.is_some(),
            "El commit local debe haberse conservado pese al fallo de red en fetch/push"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
