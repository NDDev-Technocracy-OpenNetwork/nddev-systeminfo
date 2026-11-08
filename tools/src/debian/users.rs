//! Actual independent user managers on the disposable Debian qualification VM.
use crate::{Result, command};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

pub(super) struct Users(Vec<(String, u32, PathBuf)>);
impl Users {
    pub(super) fn create() -> Result<Self> {
        let mut users = Self(Vec::new());
        for suffix in ["a", "b"] {
            let name = format!("si-deb-{}-{suffix}", std::process::id());
            let home = std::env::temp_dir().join(&name);
            if home.exists() {
                return Err("native Debian fixture home is occupied".into());
            }
            command(
                "sudo",
                &[
                    "useradd",
                    "--system",
                    "--create-home",
                    "--home-dir",
                    home.to_str().unwrap(),
                    "--shell",
                    "/usr/sbin/nologin",
                    &name,
                ],
            )?;
            let output = Command::new("id").args(["-u", &name]).output()?;
            let uid = String::from_utf8(output.stdout)?.trim().parse()?;
            users.0.push((name, uid, home));
        }
        // The enabled manager must precede the masked one in native unit order.
        users.0.sort_by_key(|(_, uid, _)| uid.to_string());
        let (name, _, home) = &users.0[1];
        let directory = home.join(".config/systemd/user");
        command(
            "sudo",
            &["-u", name, "mkdir", "-p", directory.to_str().unwrap()],
        )?;
        for unit in [super::SOCKET, "rldyour-sysinfod.service"] {
            command(
                "sudo",
                &[
                    "-u",
                    name,
                    "ln",
                    "-s",
                    "/dev/null",
                    directory.join(unit).to_str().unwrap(),
                ],
            )?;
        }
        for (_, uid, _) in &users.0 {
            command(
                "sudo",
                &["systemctl", "start", &format!("user@{uid}.service")],
            )?;
        }
        Ok(users)
    }
    pub(super) fn verify(&self) -> Result {
        for (index, (_, uid, home)) in self.0.iter().enumerate() {
            let active = Command::new("sudo")
                .args([
                    "systemctl",
                    "--user",
                    "--machine",
                    &format!("{uid}@"),
                    "--quiet",
                    "is-active",
                    super::SOCKET,
                ])
                .status()?
                .success();
            if active != (index == 0) {
                return Err("native Debian users lost independent startup admission".into());
            }
            if index == 1 {
                for unit in [super::SOCKET, "rldyour-sysinfod.service"] {
                    let link = Command::new("sudo")
                        .arg("readlink")
                        .arg(home.join(".config/systemd/user").join(unit))
                        .output()?;
                    if !link.status.success()
                        || String::from_utf8_lossy(&link.stdout).trim() != "/dev/null"
                    {
                        return Err("Debian restart changed a user's mask".into());
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn finish(mut self) -> Result {
        while let Some((name, uid, _)) = self.0.last() {
            command(
                "sudo",
                &["systemctl", "stop", &format!("user@{uid}.service")],
            )?;
            command("sudo", &["userdel", "--remove", name])?;
            self.0.pop();
        }
        Ok(())
    }
}
impl Drop for Users {
    fn drop(&mut self) {
        for (name, uid, _) in self.0.iter().rev() {
            let _ = Command::new("sudo")
                .args(["systemctl", "stop", &format!("user@{uid}.service")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("sudo")
                .args(["userdel", "--remove", name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}
