fn main() {
    // The program's icon, the manifest that declares it aware of each
    // monitor's scale, and its version as Windows shows it (file
    // properties, Task Manager): Windows resources, for a Windows build.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let root = std::env::var("CARGO_MANIFEST_DIR").unwrap().replace('\\', "/");
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let mut numbers: Vec<u16> = version.split(['.', '-']).filter_map(|part| part.parse().ok()).take(4).collect();
        numbers.resize(4, 0);
        let numbers = numbers.iter().map(u16::to_string).collect::<Vec<_>>().join(",");
        let script = format!(
            r#"#define RT_MANIFEST 24
1 ICON "{root}/icons/icon.ico"
1 RT_MANIFEST "{root}/glance.manifest"

1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "lulu-loopp"
      VALUE "FileDescription", "Glance"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "glance"
      VALUE "LegalCopyright", "lulu-loopp"
      VALUE "OriginalFilename", "glance.exe"
      VALUE "ProductName", "Glance"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
        );
        let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("glance.rc");
        std::fs::write(&path, script).unwrap();
        println!("cargo:rerun-if-changed=icons/icon.ico");
        println!("cargo:rerun-if-changed=glance.manifest");
        embed_resource::compile(&path, embed_resource::NONE).manifest_required().unwrap();
    }
}
