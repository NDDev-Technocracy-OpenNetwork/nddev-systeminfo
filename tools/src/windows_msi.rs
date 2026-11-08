//! A small per-user maintenance package delegates executable lifetime and its
//! temporary cleanup action to Windows Installer. No resident helper or shell.
use crate::{Result, digest};
use msi::{Column, Insert, Package, PackageType, Value};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};

pub(super) fn build(root: &Path, version: &str) -> Result {
    let identity: toml::Value =
        toml::from_str(include_str!("../../packaging/windows/maintenance.toml"))?;
    let code = |key: &str| -> Result<&str> {
        Ok(identity[key]
            .as_str()
            .ok_or("Windows installer identity missing")?)
    };
    let executable = root.join("bin/nddev-systeminfo.exe");
    let hash = digest(&executable)?;
    let uuid = format!(
        "{}-{}-4{}-8{}-{}",
        &hash[..8],
        &hash[8..12],
        &hash[13..16],
        &hash[17..20],
        &hash[20..32]
    );
    let mut package = Package::create(PackageType::Installer, Cursor::new(Vec::new()))?;
    package.set_database_codepage(msi::CodePage::Windows1252);
    let info = package.summary_info_mut();
    info.set_codepage(msi::CodePage::Windows1252);
    info.set_title("NDDev Systeminfo maintenance");
    info.set_author("NDDev OpenNetwork");
    info.set_arch("x64");
    info.set_languages(&[msi::LanguageId::from_id(1033)]);
    info.set_page_count(500);
    info.set_word_count(8); // msidbSumInfoSourceTypeLUAPackage: no elevation.
    info.set_uuid(uuid.parse()?);
    package.create_table(
        "Property",
        vec![
            Column::build("Property").primary_key().id_string(72),
            Column::build("Value").string(0),
        ],
    )?;
    for (key, value) in [
        ("ProductCode", code("product_code")?),
        ("UpgradeCode", code("upgrade_code")?),
        ("ProductName", "NDDev Systeminfo"),
        ("ProductVersion", version),
        ("ProductLanguage", "1033"),
        ("Manufacturer", "NDDev OpenNetwork"),
        ("INSTALLLEVEL", "1"),
        ("MSIINSTALLPERUSER", "1"),
        ("ARPNOMODIFY", "1"),
        ("ARPNOREPAIR", "1"),
        ("SYSTEMINFO_UNINSTALL_PID", "0"),
        ("SecureCustomProperties", "SYSTEMINFO_UNINSTALL_PID"),
    ] {
        package.insert_rows(Insert::into("Property").row(vec![key.into(), value.into()]))?;
    }
    package.create_table(
        "Directory",
        vec![
            Column::build("Directory").primary_key().id_string(72),
            Column::build("Directory_Parent").nullable().id_string(72),
            Column::build("DefaultDir").string(255),
        ],
    )?;
    package.insert_rows(Insert::into("Directory").row(vec![
        "TARGETDIR".into(),
        Value::Null,
        "SourceDir".into(),
    ]))?;
    package.create_table(
        "Feature",
        vec![
            Column::build("Feature").primary_key().id_string(38),
            Column::build("Feature_Parent").nullable().id_string(38),
            Column::build("Title").nullable().string(64),
            Column::build("Description").nullable().string(255),
            Column::build("Display").nullable().int16(),
            Column::build("Level").int16(),
            Column::build("Directory_").nullable().id_string(72),
            Column::build("Attributes").int16(),
        ],
    )?;
    package.insert_rows(Insert::into("Feature").row(vec![
        "Maintenance".into(),
        Value::Null,
        "Systeminfo".into(),
        Value::Null,
        1.into(),
        1.into(),
        Value::Null,
        0.into(),
    ]))?;
    package.create_table(
        "Component",
        vec![
            Column::build("Component").primary_key().id_string(72),
            Column::build("ComponentId").nullable().string(38),
            Column::build("Directory_").id_string(72),
            Column::build("Attributes").int16(),
            Column::build("Condition").nullable().string(255),
            Column::build("KeyPath").nullable().string(72),
        ],
    )?;
    package.insert_rows(Insert::into("Component").row(vec![
        "Maintenance".into(),
        code("component_code")?.into(),
        "TARGETDIR".into(),
        260.into(),
        Value::Null,
        "Maintenance".into(),
    ]))?;
    package.create_table(
        "FeatureComponents",
        vec![
            Column::build("Feature_").primary_key().id_string(38),
            Column::build("Component_").primary_key().id_string(72),
        ],
    )?;
    package.insert_rows(
        Insert::into("FeatureComponents").row(vec!["Maintenance".into(), "Maintenance".into()]),
    )?;
    package.create_table(
        "Registry",
        vec![
            Column::build("Registry").primary_key().id_string(72),
            Column::build("Root").int16(),
            Column::build("Key").string(255),
            Column::build("Name").nullable().string(255),
            Column::build("Value").nullable().string(0),
            Column::build("Component_").id_string(72),
        ],
    )?;
    package.insert_rows(Insert::into("Registry").row(vec![
        "Maintenance".into(),
        1.into(),
        "Software\\NDDev\\Systeminfo".into(),
        "WindowsInstaller".into(),
        "#1".into(),
        "Maintenance".into(),
    ]))?;
    package.create_table(
        "Binary",
        vec![
            Column::build("Name").primary_key().id_string(72),
            Column::build("Data").binary(),
        ],
    )?;
    package.insert_rows(Insert::into("Binary").row(vec!["Cleanup".into(), Value::Binary]))?;
    package
        .write_stream("Binary.Cleanup")?
        .write_all(&fs::read(executable)?)?;
    package.create_table(
        "CustomAction",
        vec![
            Column::build("Action").primary_key().id_string(72),
            Column::build("Type").int16(),
            Column::build("Source").nullable().string(72),
            Column::build("Target").nullable().string(0),
        ],
    )?;
    package.insert_rows(Insert::into("CustomAction").row(vec![
        "CleanupOwnedProduct".into(),
        1026.into(),
        "Cleanup".into(),
        "uninstall --windows-maintenance [SYSTEMINFO_UNINSTALL_PID]".into(),
    ]))?;
    package.create_table(
        "InstallExecuteSequence",
        vec![
            Column::build("Action").primary_key().id_string(72),
            Column::build("Condition").nullable().string(255),
            Column::build("Sequence").int16(),
        ],
    )?;
    for (action, sequence) in [
        ("CostInitialize", 800),
        ("FileCost", 900),
        ("CostFinalize", 1000),
        ("InstallValidate", 1400),
        ("InstallInitialize", 1500),
        ("ProcessComponents", 1600),
        ("RemoveRegistryValues", 2600),
        ("WriteRegistryValues", 5000),
        ("RegisterUser", 6000),
        ("RegisterProduct", 6100),
        ("PublishFeatures", 6300),
        ("PublishProduct", 6400),
        ("CleanupOwnedProduct", 6500),
        ("InstallFinalize", 6600),
    ] {
        package.insert_rows(Insert::into("InstallExecuteSequence").row(vec![
            action.into(),
            if action == "CleanupOwnedProduct" {
                "REMOVE=\"ALL\"".into()
            } else {
                Value::Null
            },
            sequence.into(),
        ]))?;
    }
    // msi::create uses CFB's v4 default. Windows Installer requires the v3
    // structured-storage layout, so serialize the same MSI streams to v3.
    let mut source = cfb::CompoundFile::open(package.into_inner()?)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(root.join("maintenance.msi"))?;
    let mut output = cfb::CompoundFile::create_with_version(cfb::Version::V3, file)?;
    output.set_storage_clsid("/", *source.root_entry().clsid())?;
    let streams: Vec<_> = source
        .walk()
        .filter(|entry| entry.is_stream())
        .map(|entry| entry.path().to_path_buf())
        .collect();
    for stream in streams {
        std::io::copy(
            &mut source.open_stream(&stream)?,
            &mut output.create_stream(&stream)?,
        )?;
    }
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn maintenance_embeds_the_cleanup_bytes_in_a_readable_msi() {
        let root =
            Directory(std::env::temp_dir().join(format!("systeminfo-msi-{}", std::process::id())));
        fs::create_dir_all(root.0.join("bin")).unwrap();
        let executable = b"MZ-cleanup-payload";
        fs::write(root.0.join("bin/nddev-systeminfo.exe"), executable).unwrap();
        build(&root.0, "0.3.2").unwrap();
        let header = fs::read(root.0.join("maintenance.msi")).unwrap();
        assert_eq!(&header[26..28], &[3, 0]);
        assert_eq!(&header[30..32], &[9, 0]);
        let mut package =
            Package::open(fs::File::open(root.0.join("maintenance.msi")).unwrap()).unwrap();
        let mut payload = Vec::new();
        package
            .read_stream("Binary.Cleanup")
            .unwrap()
            .read_to_end(&mut payload)
            .unwrap();
        assert_eq!(payload, executable);
        assert!(package.has_table("InstallExecuteSequence"));
    }
}
