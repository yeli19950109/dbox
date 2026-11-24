use super::{ApiErrorCode, ApiErrorDto};

#[tauri::command]
#[specta::specta]
pub async fn llama_read_api_key() -> Result<Option<String>, ApiErrorDto> {
    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(keychain::read)
            .await
            .map_err(|_| unavailable("读取钥匙串失败"))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(unavailable("保存 API Key 需要 macOS 钥匙串"))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn llama_save_api_key(value: String) -> Result<(), ApiErrorDto> {
    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(move || keychain::save(&value))
            .await
            .map_err(|_| unavailable("写入钥匙串失败"))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = value;
        Err(unavailable("保存 API Key 需要 macOS 钥匙串"))
    }
}

fn unavailable(message: impl Into<String>) -> ApiErrorDto {
    ApiErrorDto::new(ApiErrorCode::Unavailable, message, true)
}

#[cfg(target_os = "macos")]
mod keychain {
    use super::{unavailable, ApiErrorDto};
    use security_framework::{
        base::Error,
        passwords::{
            delete_generic_password, generic_password, set_generic_password, PasswordOptions,
        },
    };
    use security_framework_sys::base::errSecItemNotFound;

    const SERVICE: &str = "com.yel.dbox.llama-server";
    const ACCOUNT: &str = "api-key";

    fn keychain_error(error: Error) -> ApiErrorDto {
        unavailable(format!("无法访问 macOS 钥匙串：{error} ({})", error.code()))
    }

    fn optional_item<T>(result: Result<T, Error>) -> Result<Option<T>, ApiErrorDto> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.code() == errSecItemNotFound => Ok(None),
            Err(error) => Err(keychain_error(error)),
        }
    }

    pub(super) fn read() -> Result<Option<String>, ApiErrorDto> {
        let options = PasswordOptions::new_generic_password(SERVICE, ACCOUNT);
        optional_item(generic_password(options))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| unavailable("钥匙串中的 API Key 编码无效"))
            })
            .transpose()
    }

    pub(super) fn save(value: &str) -> Result<(), ApiErrorDto> {
        if value.is_empty() {
            return optional_item(delete_generic_password(SERVICE, ACCOUNT)).map(|_| ());
        }
        set_generic_password(SERVICE, ACCOUNT, value.as_bytes()).map_err(keychain_error)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use security_framework_sys::base::errSecAuthFailed;

        #[test]
        fn missing_key_is_absent_and_access_denied_remains_an_error() {
            assert_eq!(optional_item(Ok("fixture")).unwrap(), Some("fixture"));
            assert_eq!(
                optional_item::<()>(Err(Error::from_code(errSecItemNotFound))).unwrap(),
                None
            );
            assert!(optional_item::<()>(Err(Error::from_code(errSecAuthFailed))).is_err());
        }
    }
}
