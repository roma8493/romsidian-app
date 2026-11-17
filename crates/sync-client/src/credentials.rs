//! Gestión segura de credenciales para sincronización (`[RF-08]`).
//!
//! El Personal Access Token (PAT) de GitHub se almacena exclusivamente en el almacén
//! de credenciales del sistema operativo (`keyring` -> Windows Credential Manager) o
//! en un almacén en memoria durante pruebas automatizadas, **nunca** en archivos dentro
//! de la bóveda ni en `.romsidian/`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use crate::{SyncError, SyncResult};

static ANDROID_SECRETS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Registra el directorio privado de la aplicación en Android para resguardar tokens PAT fuera de la bóveda.
pub fn set_android_secrets_dir(dir: PathBuf) {
    let _ = ANDROID_SECRETS_DIR.set(dir);
}

/// Nombre del servicio registrado en el gestor de credenciales del sistema operativo.
pub const KEYRING_SERVICE_NAME: &str = "ROMSIDIAN_GIT_SYNC";

/// Contrato para almacenar, recuperar y eliminar tokens de autenticación de forma segura.
pub trait CredentialStore: Send + Sync {
    /// Guarda el token asociado al identificador de bóveda o repositorio.
    fn set_token(&self, account_key: &str, token: &str) -> SyncResult<()>;

    /// Obtiene el token asociado al identificador de bóveda o repositorio, si existe.
    fn get_token(&self, account_key: &str) -> SyncResult<Option<String>>;

    /// Elimina el token almacenado para el identificador indicado.
    fn delete_token(&self, account_key: &str) -> SyncResult<()>;
}

/// Almacén de credenciales respaldado por el sistema operativo (`keyring` / Windows Credential Manager).
#[derive(Debug, Clone, Default)]
pub struct OsKeyringStore {
    service_name: String,
}

impl OsKeyringStore {
    /// Crea una instancia usando el servicio estándar de ROMSIDIAN.
    pub fn new() -> Self {
        Self {
            service_name: KEYRING_SERVICE_NAME.to_string(),
        }
    }

    /// Crea una instancia con un nombre de servicio personalizado (útil para aislamiento).
    pub fn with_service(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
        }
    }
}

#[cfg(target_os = "android")]
fn android_secret_path(service_name: &str, account_key: &str) -> SyncResult<PathBuf> {
    let base = ANDROID_SECRETS_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("/data/local/tmp/.romsidian_secrets"));
    std::fs::create_dir_all(&base).map_err(|e| {
        SyncError::Credentials(format!("Error al crear directorio privado de credenciales: {e}"))
    })?;
    let raw = format!("{service_name}:{account_key}");
    let hex: String = raw.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    let short_name = if hex.len() > 64 { &hex[..64] } else { &hex };
    Ok(base.join(format!("pat_{short_name}.key")))
}

impl CredentialStore for OsKeyringStore {
    fn set_token(&self, account_key: &str, token: &str) -> SyncResult<()> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(SyncError::Credentials(
                "El token de acceso no puede estar vacío".to_string(),
            ));
        }
        #[cfg(target_os = "android")]
        {
            let path = android_secret_path(&self.service_name, account_key)?;
            std::fs::write(&path, trimmed).map_err(|e| {
                SyncError::Credentials(format!("Error al guardar token en almacenamiento privado Android: {e}"))
            })?;
            return Ok(());
        }
        #[cfg(not(target_os = "android"))]
        {
            let entry = keyring::Entry::new(&self.service_name, account_key)
                .map_err(|e| SyncError::Credentials(format!("Error al abrir keyring del SO: {e}")))?;
            entry
                .set_password(trimmed)
                .map_err(|e| SyncError::Credentials(format!("Error al guardar token en el SO: {e}")))
        }
    }

    fn get_token(&self, account_key: &str) -> SyncResult<Option<String>> {
        #[cfg(target_os = "android")]
        {
            let path = android_secret_path(&self.service_name, account_key)?;
            if !path.exists() {
                return Ok(None);
            }
            let content = std::fs::read_to_string(&path).map_err(|e| {
                SyncError::Credentials(format!("Error al leer token en almacenamiento privado Android: {e}"))
            })?;
            let trimmed = content.trim().to_string();
            return Ok(if trimmed.is_empty() { None } else { Some(trimmed) });
        }
        #[cfg(not(target_os = "android"))]
        {
            let entry = keyring::Entry::new(&self.service_name, account_key)
                .map_err(|e| SyncError::Credentials(format!("Error al abrir keyring del SO: {e}")))?;
            match entry.get_password() {
                Ok(pwd) => Ok(Some(pwd)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(SyncError::Credentials(format!(
                    "Error al leer token del SO: {e}"
                ))),
            }
        }
    }

    fn delete_token(&self, account_key: &str) -> SyncResult<()> {
        #[cfg(target_os = "android")]
        {
            let path = android_secret_path(&self.service_name, account_key)?;
            if path.exists() {
                let _ = std::fs::remove_file(path);
            }
            return Ok(());
        }
        #[cfg(not(target_os = "android"))]
        {
            let entry = keyring::Entry::new(&self.service_name, account_key)
                .map_err(|e| SyncError::Credentials(format!("Error al abrir keyring del SO: {e}")))?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(SyncError::Credentials(format!(
                    "Error al eliminar token del SO: {e}"
                ))),
            }
        }
    }
}

/// Almacén de credenciales efímero en memoria RAM (para pruebas automatizadas y entornos sin UI).
#[derive(Debug, Clone, Default)]
pub struct MemoryCredentialStore {
    tokens: Arc<Mutex<HashMap<String, String>>>,
}

impl MemoryCredentialStore {
    /// Crea un almacén en memoria vacío.
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialStore for MemoryCredentialStore {
    fn set_token(&self, account_key: &str, token: &str) -> SyncResult<()> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(SyncError::Credentials(
                "El token de acceso no puede estar vacío".to_string(),
            ));
        }
        let mut guard = self
            .tokens
            .lock()
            .map_err(|_| SyncError::Credentials("Mutex de credenciales envenenado".to_string()))?;
        guard.insert(account_key.to_string(), trimmed.to_string());
        Ok(())
    }

    fn get_token(&self, account_key: &str) -> SyncResult<Option<String>> {
        let guard = self
            .tokens
            .lock()
            .map_err(|_| SyncError::Credentials("Mutex de credenciales envenenado".to_string()))?;
        Ok(guard.get(account_key).cloned())
    }

    fn delete_token(&self, account_key: &str) -> SyncResult<()> {
        let mut guard = self
            .tokens
            .lock()
            .map_err(|_| SyncError::Credentials("Mutex de credenciales envenenado".to_string()))?;
        guard.remove(account_key);
        Ok(())
    }
}
