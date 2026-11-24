use super::{ApiErrorCode, ApiErrorDto};

#[tauri::command]
#[specta::specta]
pub async fn llama_read_api_key(profile_id: Option<String>) -> Result<Option<String>, ApiErrorDto> {
    #[cfg(target_os = "macos")]
    {
        let account = keychain::account(profile_id.as_deref())?;
        tokio::task::spawn_blocking(move || keychain::read(&account))
            .await
            .map_err(|_| unavailable("读取钥匙串失败"))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = profile_id;
        Err(unavailable("保存 API Key 需要 macOS 钥匙串"))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn llama_save_api_key(
    value: String,
    profile_id: Option<String>,
) -> Result<(), ApiErrorDto> {
    #[cfg(target_os = "macos")]
    {
        let account = keychain::account(profile_id.as_deref())?;
        tokio::task::spawn_blocking(move || keychain::save(&account, &value))
            .await
            .map_err(|_| unavailable("写入钥匙串失败"))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (value, profile_id);
        Err(unavailable("保存 API Key 需要 macOS 钥匙串"))
    }
}

fn unavailable(message: impl Into<String>) -> ApiErrorDto {
    ApiErrorDto::new(ApiErrorCode::Unavailable, message, true)
}

#[cfg(target_os = "macos")]
mod keychain {
    use super::{unavailable, ApiErrorCode, ApiErrorDto};
    use security_framework::{
        base::Error,
        passwords::{
            delete_generic_password, generic_password, set_generic_password, PasswordOptions,
        },
    };
    use security_framework_sys::base::errSecItemNotFound;

    const SERVICE: &str = "com.yel.dbox.llama-server";
    // v0.3.1 stored the shared key here. The default profile keeps that exact item.
    const DEFAULT_ACCOUNT: &str = "api-key";

    pub(super) fn account(profile_id: Option<&str>) -> Result<String, ApiErrorDto> {
        match profile_id {
            None | Some("default") => Ok(DEFAULT_ACCOUNT.into()),
            Some(id) => id
                .parse::<uuid::Uuid>()
                .ok()
                .filter(|_| id.len() == 36)
                .map(|uuid| format!("{DEFAULT_ACCOUNT}/{uuid}"))
                .ok_or_else(|| {
                    ApiErrorDto::new(ApiErrorCode::InvalidRequest, "无效的配置 ID", false)
                }),
        }
    }

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

    pub(super) fn read(account: &str) -> Result<Option<String>, ApiErrorDto> {
        let options = PasswordOptions::new_generic_password(SERVICE, account);
        optional_item(generic_password(options))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| unavailable("钥匙串中的 API Key 编码无效"))
            })
            .transpose()
    }

    pub(super) fn save(account: &str, value: &str) -> Result<(), ApiErrorDto> {
        if value.is_empty() {
            return optional_item(delete_generic_password(SERVICE, account)).map(|_| ());
        }
        set_generic_password(SERVICE, account, value.as_bytes()).map_err(keychain_error)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use security_framework_sys::base::errSecAuthFailed;

        #[test]
        fn default_profile_reuses_the_existing_key_and_other_profiles_have_separate_accounts() {
            assert_eq!(SERVICE, "com.yel.dbox.llama-server");
            assert_eq!(account(None).unwrap(), "api-key");
            assert_eq!(account(Some("default")).unwrap(), "api-key");
            let first = "21964aa1-61be-4761-97fd-503fe21affb0";
            let second = "21964aa1-61be-4761-97fd-503fe21affb1";
            assert_eq!(account(Some(first)).unwrap(), format!("api-key/{first}"));
            assert_ne!(
                account(Some(first)).unwrap(),
                account(Some(second)).unwrap()
            );
            assert_eq!(
                account(Some(&first.to_uppercase())).unwrap(),
                account(Some(first)).unwrap()
            );
            assert!(account(Some("")).is_err());
            assert!(account(Some("profile name")).is_err());
        }

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
