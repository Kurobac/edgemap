use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};

use crate::shutdown::unblock_shutdown_signals_in_child;

use super::monitor::associated_input_nodes;

pub(super) struct NodePermissions {
    sysfs_path: Option<PathBuf>,
    restored_nodes: Vec<RestoredNode>,
}

struct RestoredNode {
    path: PathBuf,
    mode: u32,
    acl: String,
}

impl NodePermissions {
    pub(super) fn new(sysfs_path: Option<PathBuf>) -> Self {
        Self {
            sysfs_path,
            restored_nodes: Vec::new(),
        }
    }

    fn read_acl(path: &Path) -> io::Result<String> {
        let mut command = std::process::Command::new("getfacl");
        command.args(["--absolute-names", &path.to_string_lossy()]);
        unblock_shutdown_signals_in_child(&mut command);
        let output = command.output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "getfacl failed with {}",
                output.status
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn clear_acl(path: &Path) {
        let mut command = std::process::Command::new("setfacl");
        command.args(["-b", &path.to_string_lossy()]);
        unblock_shutdown_signals_in_child(&mut command);
        match command.output() {
            Ok(output) if output.status.success() => {}
            Ok(output) => warn!(
                "failed to clear node ACL: path={}, status={}",
                path.display(),
                output.status
            ),
            Err(e) => warn!(
                "failed to clear node ACL: path={}, error={e}",
                path.display()
            ),
        }
    }

    fn restrict_node(path: &Path, restored: &mut Vec<RestoredNode>) -> io::Result<()> {
        let mode = fs::metadata(path)?.permissions().mode();
        let acl = Self::read_acl(path).unwrap_or_else(|e| {
            warn!(
                "failed to capture node ACL: path={}, error={e}",
                path.display()
            );
            String::new()
        });

        restored.push(RestoredNode {
            path: path.to_path_buf(),
            mode,
            acl,
        });

        Self::clear_acl(path);
        fs::set_permissions(path, fs::Permissions::from_mode(0o000))?;
        Ok(())
    }

    pub(super) fn restrict(&mut self) -> io::Result<()> {
        let mut hidden = Vec::new();
        let mut failures = Vec::new();

        if let Some(ref sysfs) = self.sysfs_path {
            let devname = sysfs
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let hidraw_path = PathBuf::from("/dev").join(devname);
            if hidraw_path.exists() {
                match Self::restrict_node(&hidraw_path, &mut self.restored_nodes) {
                    Ok(()) => hidden.push(devname.to_string()),
                    Err(error) => failures.push(format!(
                        "failed to restrict {}: {error}",
                        hidraw_path.display()
                    )),
                }
            }

            match associated_input_nodes(sysfs) {
                Ok(nodes) => {
                    for node in nodes {
                        if let Some(dev_path) = node.dev_path.filter(|path| path.exists()) {
                            match Self::restrict_node(&dev_path, &mut self.restored_nodes) {
                                Ok(()) => hidden.push(node.name),
                                Err(error) => failures.push(format!(
                                    "failed to restrict {}: {error}",
                                    dev_path.display()
                                )),
                            }
                        }
                    }
                }
                Err(error) => failures.push(format!(
                    "failed to enumerate associated input nodes: {error}"
                )),
            }
        }

        info!("input nodes restricted: count={}", hidden.len());
        for name in &hidden {
            debug!("input node restricted: path={name}");
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(failures.join("; ")))
        }
    }

    pub(super) fn forget(&mut self) {
        self.restored_nodes.clear();
    }

    pub(super) fn re_restrict(&mut self) {
        let mut restricted = Vec::new();
        for node in &mut self.restored_nodes {
            let mode = match fs::metadata(&node.path) {
                Ok(metadata) => metadata.permissions().mode(),
                Err(e) => {
                    warn!(
                        "failed to inspect node permissions: path={}, error={e}",
                        node.path.display()
                    );
                    continue;
                }
            };
            if mode & 0o777 == 0 {
                continue;
            }

            match Self::read_acl(&node.path) {
                Ok(acl) => {
                    node.mode = mode;
                    node.acl = acl;
                }
                Err(e) => {
                    warn!(
                        "failed to refresh node ACL snapshot; previous snapshot retained: path={}, error={e}",
                        node.path.display()
                    );
                }
            }

            Self::clear_acl(&node.path);
            match fs::set_permissions(&node.path, fs::Permissions::from_mode(0o000)) {
                Ok(()) => restricted.push(node.path.clone()),
                Err(e) => warn!(
                    "failed to re-restrict input node: path={}, error={e}",
                    node.path.display()
                ),
            }
        }

        if !restricted.is_empty() {
            info!(
                "input nodes re-restricted after permission reset: count={}",
                restricted.len()
            );
            for path in restricted {
                debug!("input node re-restricted: path={}", path.display());
            }
        }
    }

    fn restore(&self) {
        if self.restored_nodes.is_empty() {
            return;
        }
        info!(
            "restoring input node permissions: count={}",
            self.restored_nodes.len()
        );
        let mut acl_batch = String::new();
        for node in &self.restored_nodes {
            if !node.path.exists() {
                continue;
            }
            if let Err(e) = fs::set_permissions(&node.path, fs::Permissions::from_mode(node.mode)) {
                warn!(
                    "failed to restore node permissions: path={}, error={e}",
                    node.path.display()
                );
            } else {
                debug!("node permissions restored: path={}", node.path.display());
            }

            if !node.acl.is_empty() {
                acl_batch.push_str(&node.acl);
            }
        }

        if !acl_batch.is_empty() {
            let mut command = std::process::Command::new("setfacl");
            command
                .arg("-P")
                .arg("--restore=-")
                .stdin(std::process::Stdio::piped());
            unblock_shutdown_signals_in_child(&mut command);
            match command.spawn() {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        if let Err(e) = stdin.write_all(acl_batch.as_bytes()) {
                            warn!("failed to write ACL restore data to setfacl: {e}");
                        }
                    }
                    match child.wait() {
                        Ok(status) if status.success() => {}
                        Ok(status) => warn!("failed to restore ACLs with setfacl: status={status}"),
                        Err(e) => warn!("failed to wait for setfacl ACL restore: {e}"),
                    }
                }
                Err(e) => warn!("failed to start setfacl ACL restore: {e}"),
            }
        }
    }
}

impl Drop for NodePermissions {
    fn drop(&mut self) {
        self.restore();
    }
}
