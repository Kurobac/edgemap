use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;

use super::Config;

pub const MAX_CONFIG_FILE_SIZE: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveConfig {
    source: String,
    content: String,
}

impl ActiveConfig {
    pub fn read(path: &str) -> Result<Self, String> {
        let content = read_config_content(path)?;
        Ok(Self {
            source: path.to_string(),
            content,
        })
    }

    pub fn from_content(source: String, content: String) -> Result<Self, String> {
        if content.len() > MAX_CONFIG_FILE_SIZE {
            return Err(format!(
                "Config content exceeds {MAX_CONFIG_FILE_SIZE} byte limit"
            ));
        }
        Ok(Self { source, content })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn parse(&self) -> Result<Config, String> {
        Config::parse(&self.source, &self.content)
    }
}

fn read_config_content(path: &str) -> Result<String, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("Cannot open {path}: {e}"))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Cannot inspect {path}: {e}"))?;
    if !metadata.file_type().is_file() {
        return Err(format!("Config path is not a regular file: {path}"));
    }
    if metadata.len() > MAX_CONFIG_FILE_SIZE as u64 {
        return Err(format!(
            "Config file exceeds {MAX_CONFIG_FILE_SIZE} byte limit: {path}"
        ));
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_CONFIG_FILE_SIZE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read {path}: {e}"))?;
    if bytes.len() > MAX_CONFIG_FILE_SIZE {
        return Err(format!(
            "Config file exceeds {MAX_CONFIG_FILE_SIZE} byte limit: {path}"
        ));
    }
    String::from_utf8(bytes).map_err(|e| format!("Invalid UTF-8 in config {path}: {e}"))
}
