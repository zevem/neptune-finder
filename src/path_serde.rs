//! Unicode paths stay readable in preferences. Native non-Unicode paths retain
//! their exact bytes/code units instead of failing a save or changing identity.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum NativePath {
    Text(String),
    Unix { unix_bytes: Vec<u8> },
    Windows { windows_units: Vec<u16> },
}
fn encode(path: &std::path::Path) -> NativePath {
    if let Some(text) = path.to_str() {
        return NativePath::Text(text.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        NativePath::Unix {
            unix_bytes: path.as_os_str().as_bytes().to_vec(),
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        NativePath::Windows {
            windows_units: path.as_os_str().encode_wide().collect(),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        NativePath::Text(path.to_string_lossy().into_owned())
    }
}
fn decode(path: NativePath) -> Result<PathBuf, String> {
    match path {
        NativePath::Text(text) => Ok(text.into()),
        NativePath::Unix { unix_bytes } => {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                Ok(std::ffi::OsString::from_vec(unix_bytes).into())
            }
            #[cfg(not(unix))]
            {
                let _ = unix_bytes;
                Err(
                    "This session contains a Unix path that cannot be represented on this platform"
                        .into(),
                )
            }
        }
        NativePath::Windows { windows_units } => {
            #[cfg(windows)]
            {
                use std::os::windows::ffi::OsStringExt;
                Ok(std::ffi::OsString::from_wide(&windows_units).into())
            }
            #[cfg(not(windows))]
            {
                let _ = windows_units;
                Err("This session contains a Windows path that cannot be represented on this platform".into())
            }
        }
    }
}
pub mod many {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        paths: &[PathBuf],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        paths
            .iter()
            .map(|p| encode(p))
            .collect::<Vec<_>>()
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<PathBuf>, D::Error> {
        Vec::<NativePath>::deserialize(deserializer)?
            .into_iter()
            .map(decode)
            .collect::<Result<_, _>>()
            .map_err(serde::de::Error::custom)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Serialize, Deserialize)]
    struct Paths {
        #[serde(with = "many")]
        paths: Vec<PathBuf>,
    }
    #[cfg(unix)]
    #[test]
    fn native_non_utf8_path_survives_json_and_toml() {
        use std::os::unix::ffi::OsStringExt;
        let paths = Paths {
            paths: vec![
                "/Documents".into(),
                std::ffi::OsString::from_vec(b"/folder/\xff".to_vec()).into(),
            ],
        };
        let json = serde_json::to_string(&paths).unwrap();
        let restored: Paths = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.paths, paths.paths);
        let toml = toml::to_string(&paths).unwrap();
        let restored: Paths = toml::from_str(&toml).unwrap();
        assert_eq!(restored.paths, paths.paths);
    }
}
