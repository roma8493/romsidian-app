//! Sincronización GitHub vía API REST sobre `ureq` + `rustls` para Android (`#[cfg(target_os = "android")]`).
//!
//! Evita la dependencia de OpenSSL en compilación cruzada manteniendo compatibilidad 100% con
//! commits Git estándar de escritorio (`git2`) y preservando la regla de cero pérdida de datos
//! (`nota (conflicto <dispositivo> <fecha>).md`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use git2::{ObjectType, Oid};
use romsidian_core::VAULT_METADATA_DIR;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::git_backend::build_conflict_copy_path;
use crate::{MAX_SYNC_FILE_SIZE_BYTES, SyncConfig, SyncError, SyncReport, SyncResult, SyncState};

const ANDROID_SYNC_STATE_FILE: &str = "android-sync-state.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct AndroidSyncState {
    files: BTreeMap<String, String>,
}

fn parse_github_owner_repo(remote_url: &str) -> SyncResult<(String, String)> {
    let trimmed = remote_url.trim().trim_end_matches('/');
    let without_git = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let path_part = without_git
        .strip_prefix("https://github.com/")
        .or_else(|| without_git.strip_prefix("http://github.com/"))
        .ok_or_else(|| {
            SyncError::Backend(
                "En Android solo se admite sincronización HTTPS con repositorios de github.com"
                    .to_string(),
            )
        })?;
    let mut parts = path_part.split('/').filter(|s| !s.is_empty());
    let owner = parts
        .next()
        .ok_or_else(|| SyncError::Backend("URL de GitHub inválida (falta usuario)".to_string()))?;
    let repo = parts
        .next()
        .ok_or_else(|| SyncError::Backend("URL de GitHub inválida (falta repositorio)".to_string()))?;
    Ok((owner.to_string(), repo.to_string()))
}

fn github_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build()
}

fn api_get_json(agent: &ureq::Agent, url: &str, token: &str) -> SyncResult<Option<Value>> {
    let resp = agent
        .get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "ROMSIDIAN-Android/0.2")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call();

    match resp {
        Ok(r) => {
            let val: Value = r
                .into_json()
                .map_err(|e| SyncError::Network(format!("Respuesta JSON inválida de GitHub: {e}")))?;
            Ok(Some(val))
        }
        Err(ureq::Error::Status(404, _)) | Err(ureq::Error::Status(409, _)) => Ok(None),
        Err(ureq::Error::Status(code, r)) => {
            let msg = r.into_string().unwrap_or_default();
            Err(SyncError::Network(format!("GitHub API HTTP {code}: {msg}")))
        }
        Err(e) => Err(SyncError::Network(format!("Error de red con GitHub: {e}"))),
    }
}

fn api_post_json(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    token: &str,
    body: &Value,
) -> SyncResult<Value> {
    let req = match method {
        "PATCH" => agent.request("PATCH", url),
        _ => agent.post(url),
    };
    let resp = req
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "ROMSIDIAN-Android/0.2")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .send_json(body);

    match resp {
        Ok(r) => r
            .into_json()
            .map_err(|e| SyncError::Network(format!("Respuesta JSON inválida de GitHub: {e}"))),
        Err(ureq::Error::Status(code, r)) => {
            let msg = r.into_string().unwrap_or_default();
            Err(SyncError::Network(format!("GitHub API HTTP {code}: {msg}")))
        }
        Err(e) => Err(SyncError::Network(format!("Error de red con GitHub: {e}"))),
    }
}

fn collect_vault_files(vault_root: &Path) -> SyncResult<BTreeMap<String, (String, Vec<u8>)>> {
    let mut out = BTreeMap::new();
    collect_dir_recursive(vault_root, vault_root, &mut out)?;
    Ok(out)
}

fn collect_dir_recursive(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, (String, Vec<u8>)>,
) -> SyncResult<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".git" || name_str == VAULT_METADATA_DIR {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_dir_recursive(root, &path, out)?;
        } else if path.is_file() {
            let meta = entry.metadata()?;
            let rel = path
                .strip_prefix(root)
                .map_err(|e| SyncError::Backend(e.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            if meta.len() > MAX_SYNC_FILE_SIZE_BYTES {
                return Err(SyncError::FileTooLarge {
                    path: rel,
                    size_bytes: meta.len(),
                });
            }
            let bytes = fs::read(&path)?;
            let oid = Oid::hash_object(ObjectType::Blob, &bytes)?;
            out.insert(rel, (oid.to_string(), bytes));
        }
    }
    Ok(())
}

fn load_sync_state(vault_root: &Path) -> AndroidSyncState {
    let path = vault_root
        .join(VAULT_METADATA_DIR)
        .join(ANDROID_SYNC_STATE_FILE);
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_sync_state(vault_root: &Path, files: &BTreeMap<String, String>) -> SyncResult<()> {
    let meta_dir = vault_root.join(VAULT_METADATA_DIR);
    fs::create_dir_all(&meta_dir)?;
    let path = meta_dir.join(ANDROID_SYNC_STATE_FILE);
    let state = AndroidSyncState {
        files: files.clone(),
    };
    fs::write(path, serde_json::to_string_pretty(&state).unwrap_or_default())?;
    Ok(())
}

/// Ejecuta sincronización bidireccional contra GitHub REST API desde Android.
pub fn sync_github_android(
    vault_root: &Path,
    config: &SyncConfig,
    token: Option<String>,
    timestamp_label: &str,
) -> SyncResult<SyncReport> {
    let token = token
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            SyncError::Credentials(
                "Ingresa tu Personal Access Token (PAT) de GitHub para sincronizar desde Android"
                    .to_string(),
            )
        })?;

    let (owner, repo) = parse_github_owner_repo(&config.remote_url)?;
    let branch = if config.branch.trim().is_empty() {
        "main"
    } else {
        config.branch.trim()
    };
    let date_only = timestamp_label
        .split_whitespace()
        .next()
        .unwrap_or(timestamp_label);

    let agent = github_agent();
    let api_base = format!("https://api.github.com/repos/{owner}/{repo}");

    // 1. Consultar referencia de la rama remota
    let ref_url = format!("{api_base}/git/ref/heads/{branch}");
    let ref_json = api_get_json(&agent, &ref_url, &token)?;

    let mut remote_commit_sha: Option<String> = None;
    let mut remote_files: BTreeMap<String, String> = BTreeMap::new();

    if let Some(ref_val) = ref_json {
        if let Some(c_sha) = ref_val["object"]["sha"].as_str() {
            remote_commit_sha = Some(c_sha.to_string());
            let commit_url = format!("{api_base}/git/commits/{c_sha}");
            if let Some(commit_val) = api_get_json(&agent, &commit_url, &token)? {
                if let Some(tree_sha) = commit_val["tree"]["sha"].as_str() {
                    let tree_url = format!("{api_base}/git/trees/{tree_sha}?recursive=1");
                    if let Some(tree_val) = api_get_json(&agent, &tree_url, &token)? {
                        if let Some(items) = tree_val["tree"].as_array() {
                            for item in items {
                                if item["type"].as_str() == Some("blob") {
                                    if let (Some(p), Some(s)) =
                                        (item["path"].as_str(), item["sha"].as_str())
                                    {
                                        if !p.starts_with(".romsidian/") {
                                            remote_files.insert(p.to_string(), s.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let last_state = load_sync_state(vault_root);
    let mut local_files = collect_vault_files(vault_root)?;
    let mut merged_cleanly = Vec::new();
    let mut conflict_copies_created = Vec::new();

    // 2. Pull y detección de conflictos 3-vías
    for (path, remote_sha) in &remote_files {
        let local_sha = local_files.get(path).map(|(s, _)| s.clone());
        if local_sha.as_deref() == Some(remote_sha.as_str()) {
            continue;
        }

        let base_sha = last_state.files.get(path).cloned();

        // Si fue borrado localmente desde la última sincronización, respetamos el borrado para el push
        if local_sha.is_none() && base_sha.as_deref() == Some(remote_sha.as_str()) {
            continue;
        }

        // Descargar contenido remoto
        let blob_url = format!("{api_base}/git/blobs/{remote_sha}");
        let blob_val = api_get_json(&agent, &blob_url, &token)?.ok_or_else(|| {
            SyncError::Network(format!("No se pudo descargar el objeto remoto: {path}"))
        })?;
        let encoded = blob_val["content"].as_str().unwrap_or("");
        let clean_b64: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
        let remote_bytes = BASE64.decode(clean_b64).map_err(|e| {
            SyncError::Network(format!("Error al decodificar base64 de {path}: {e}"))
        })?;

        let local_unchanged = match (&local_sha, &base_sha) {
            (None, _) => true,
            (Some(ls), Some(bs)) => ls == bs,
            (Some(_), None) => false,
        };

        let target_path = vault_root.join(path);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if local_unchanged {
            fs::write(&target_path, &remote_bytes)?;
            merged_cleanly.push(path.clone());
        } else {
            // Conflicto: guardar copia local íntegra y escribir versión remota
            let conflict_rel =
                build_conflict_copy_path(vault_root, path, &config.device_name, date_only);
            let conflict_abs = vault_root.join(&conflict_rel);
            if let Some(parent) = conflict_abs.parent() {
                fs::create_dir_all(parent)?;
            }
            if let Some((_, local_bytes)) = local_files.get(path) {
                fs::write(&conflict_abs, local_bytes)?;
                conflict_copies_created.push(conflict_rel);
            }
            fs::write(&target_path, &remote_bytes)?;
        }
    }

    // Borrar archivos locales sin cambios que fueron eliminados en el remoto
    for (path, base_sha) in &last_state.files {
        if !remote_files.contains_key(path) {
            if let Some((local_sha, _)) = local_files.get(path) {
                if local_sha == base_sha {
                    let target_path = vault_root.join(path);
                    if target_path.exists() {
                        let _ = fs::remove_file(target_path);
                    }
                }
            }
        }
    }

    // 3. Re-escanear bóveda y hacer push si hay diferencias contra el remoto
    local_files = collect_vault_files(vault_root)?;
    let local_shas: BTreeMap<String, String> = local_files
        .iter()
        .map(|(k, (s, _))| (k.clone(), s.clone()))
        .collect();

    let mut pushed = false;
    let mut head_commit = remote_commit_sha.clone();

    let needs_push = local_shas != remote_files || (remote_commit_sha.is_none() && !local_shas.is_empty());

    if needs_push {
        let mut tree_entries = Vec::new();
        for (path, (sha, bytes)) in &local_files {
            let final_sha = if remote_files.get(path) == Some(sha) {
                sha.clone()
            } else {
                let blob_resp = api_post_json(
                    &agent,
                    "POST",
                    &format!("{api_base}/git/blobs"),
                    &token,
                    &json!({
                        "content": BASE64.encode(bytes),
                        "encoding": "base64"
                    }),
                )?;
                blob_resp["sha"]
                    .as_str()
                    .ok_or_else(|| SyncError::Network("GitHub no devolvió SHA de blob".to_string()))?
                    .to_string()
            };
            tree_entries.push(json!({
                "path": path,
                "mode": "100644",
                "type": "blob",
                "sha": final_sha
            }));
        }

        let tree_resp = api_post_json(
            &agent,
            "POST",
            &format!("{api_base}/git/trees"),
            &token,
            &json!({ "tree": tree_entries }),
        )?;
        let new_tree_sha = tree_resp["sha"]
            .as_str()
            .ok_or_else(|| SyncError::Network("GitHub no devolvió SHA de tree".to_string()))?;

        let parents: Vec<String> = remote_commit_sha.iter().cloned().collect();
        let commit_msg = format!(
            "ROMSIDIAN sync ({}) {}",
            config.device_name, timestamp_label
        );
        let commit_resp = api_post_json(
            &agent,
            "POST",
            &format!("{api_base}/git/commits"),
            &token,
            &json!({
                "message": commit_msg,
                "tree": new_tree_sha,
                "parents": parents,
                "author": {
                    "name": config.author_name,
                    "email": config.author_email
                }
            }),
        )?;
        let new_commit_sha = commit_resp["sha"]
            .as_str()
            .ok_or_else(|| SyncError::Network("GitHub no devolvió SHA de commit".to_string()))?
            .to_string();

        if remote_commit_sha.is_some() {
            api_post_json(
                &agent,
                "PATCH",
                &format!("{api_base}/git/refs/heads/{branch}"),
                &token,
                &json!({ "sha": new_commit_sha, "force": false }),
            )?;
        } else {
            api_post_json(
                &agent,
                "POST",
                &format!("{api_base}/git/refs"),
                &token,
                &json!({
                    "ref": format!("refs/heads/{branch}"),
                    "sha": new_commit_sha
                }),
            )?;
        }

        head_commit = Some(new_commit_sha);
        pushed = true;
    }

    save_sync_state(vault_root, &local_shas)?;

    let state = if conflict_copies_created.is_empty() {
        SyncState::UpToDate
    } else {
        SyncState::ConflictDetected {
            conflict_files: conflict_copies_created.clone(),
        }
    };

    let all_changed: BTreeSet<String> = merged_cleanly.iter().cloned().collect();

    Ok(SyncReport {
        state,
        committed_files: usize::from(pushed),
        merged_cleanly: all_changed.into_iter().collect(),
        conflict_copies_created,
        pushed,
        head_commit,
    })
}
