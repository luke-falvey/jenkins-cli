//! Reads basic-auth credentials from `~/.cloudbees_credentials`.
//!
//! The file holds a single line in the form `username:password`. The password
//! can be a Jenkins API token, which is the recommended choice.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Name of the credentials file in the user's home directory.
pub const CREDENTIALS_FILE_NAME: &str = ".cloudbees_credentials";

#[derive(Debug, Error)]
pub enum CredentialsError {
    #[error("could not find your home directory; pass --credentials-file instead")]
    NoHomeDir,
    #[error("could not read credentials file {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "credentials file {path} is invalid: {reason} (expected a single line `username:password`)"
    )]
    Invalid { path: PathBuf, reason: &'static str },
}

/// A username and password (or API token) for HTTP basic authentication.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

// Keep the password out of logs and panic messages.
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl Credentials {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }

    /// Default location: `~/.cloudbees_credentials`.
    pub fn default_path() -> Result<PathBuf, CredentialsError> {
        std::env::home_dir()
            .filter(|home| !home.as_os_str().is_empty())
            .map(|home| home.join(CREDENTIALS_FILE_NAME))
            .ok_or(CredentialsError::NoHomeDir)
    }

    /// Reads and parses a credentials file.
    pub fn from_file(path: &Path) -> Result<Self, CredentialsError> {
        let contents = fs::read_to_string(path).map_err(|source| CredentialsError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&contents).map_err(|reason| CredentialsError::Invalid {
            path: path.to_path_buf(),
            reason,
        })
    }

    /// Parses `username:password`.
    ///
    /// Splits on the first `:`, because usernames cannot contain one in basic
    /// auth but passwords can. Surrounding whitespace and a trailing newline
    /// are ignored. Blank lines and lines starting with `#` are skipped.
    pub fn parse(contents: &str) -> Result<Self, &'static str> {
        let mut lines = contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'));
        let line = lines.next().ok_or("the file is empty")?;
        if lines.next().is_some() {
            return Err("it has more than one line");
        }
        let (username, password) = line.split_once(':').ok_or("no `:` separator")?;
        if username.is_empty() {
            return Err("the username is empty");
        }
        if password.is_empty() {
            return Err("the password is empty");
        }
        Ok(Self::new(username, password))
    }
}

/// Returns a warning if other users can read the credentials file.
#[cfg(unix)]
pub fn permissions_warning(path: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path).ok()?.permissions().mode();
    (mode & 0o077 != 0).then(|| {
        format!(
            "warning: {} can be read by other users (mode {:o}); run `chmod 600 {}`",
            path.display(),
            mode & 0o777,
            path.display()
        )
    })
}

#[cfg(not(unix))]
pub fn permissions_warning(_path: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_username_and_password() {
        let creds = Credentials::parse("alice:s3cret\n").unwrap();
        assert_eq!(creds, Credentials::new("alice", "s3cret"));
    }

    #[test]
    fn password_may_contain_colons() {
        let creds = Credentials::parse("alice:a:b:c").unwrap();
        assert_eq!(creds.password, "a:b:c");
    }

    #[test]
    fn ignores_whitespace_comments_and_blank_lines() {
        let creds = Credentials::parse("# my token\n\n  bob:11aa22bb  \r\n\n").unwrap();
        assert_eq!(creds, Credentials::new("bob", "11aa22bb"));
    }

    #[test]
    fn rejects_bad_contents() {
        assert_eq!(Credentials::parse(""), Err("the file is empty"));
        assert_eq!(
            Credentials::parse("\n# only a comment\n"),
            Err("the file is empty")
        );
        assert_eq!(Credentials::parse("alice"), Err("no `:` separator"));
        assert_eq!(Credentials::parse(":pw"), Err("the username is empty"));
        assert_eq!(Credentials::parse("alice:"), Err("the password is empty"));
        assert_eq!(
            Credentials::parse("a:b\nc:d"),
            Err("it has more than one line")
        );
    }

    #[test]
    fn debug_output_hides_password() {
        let debug = format!("{:?}", Credentials::new("alice", "s3cret"));
        assert!(debug.contains("alice"));
        assert!(!debug.contains("s3cret"));
    }

    #[test]
    fn reads_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CREDENTIALS_FILE_NAME);
        fs::write(&path, "alice:token\n").unwrap();
        assert_eq!(
            Credentials::from_file(&path).unwrap(),
            Credentials::new("alice", "token")
        );
    }

    #[test]
    fn missing_file_names_the_path() {
        let err = Credentials::from_file(Path::new("/nonexistent/creds")).unwrap_err();
        assert!(err.to_string().contains("/nonexistent/creds"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn warns_about_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("creds");
        fs::write(&path, "a:b").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(permissions_warning(&path), None);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(permissions_warning(&path).unwrap().contains("chmod 600"));
    }
}
