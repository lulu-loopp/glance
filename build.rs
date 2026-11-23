fn main() {
    // Each program's icon, the manifest that declares it aware of each monitor's scale (see below), and its name and
    // version as Windows shows them (file properties, Task Manager): Windows resources, for a Windows build.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=icons/icon.ico");
        println!("cargo:rerun-if-changed=glance.manifest");
        // Glance itself, and its sensors. (One name each: Windows shows the first a file has, whatever its
        // language.)
        resources("glance", "Glance");
        resources("glance-sensors", "Glance Sensors");
        // The manifest, the same for every program, the tests' too: what they draw and read of the screens is as
        // Glance has it (aware of each monitor's scale).
        let root = std::env::var("CARGO_MANIFEST_DIR").unwrap().replace('\\', "/");
        let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("manifest.rc");
        std::fs::write(&path, format!("#pragma code_page(65001)\n#define RT_MANIFEST 24\n1 RT_MANIFEST \"{root}/glance.manifest\"\n")).unwrap();
        embed_resource::compile_for_everything(&path, embed_resource::NONE).manifest_required().unwrap();
    }
    // The libraries not every Windows has, each loaded when first called rather than as a program starts (which a
    // Windows without one would refuse): every program's, the tests' too.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rerun-if-changed=src/system_libraries/optional.rs");
        for optional in Optional::ALL {
            println!("cargo:rustc-link-arg=/DELAYLOAD:{}", optional.file());
        }
        println!("cargo:rustc-link-arg=delayimp.lib");
    }
}

include!("src/system_libraries/optional.rs");

/// The resources of the program `bin`, as `description` names it.
fn resources(bin: &str, description: &str) {
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap().replace('\\', "/");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    // Major, minor and patch, before any pre-release or build suffix.
    let core = version.split(['-', '+']).next().unwrap();
    let mut numbers: Vec<u16> = core.split('.').map(|part| part.parse().unwrap()).collect();
    numbers.resize(4, 0);
    let numbers = numbers.iter().map(u16::to_string).collect::<Vec<_>>().join(",");
    let script = format!(
        // The script is UTF-8: so are the paths in it, whatever language the
        // checkout's folders are named in.
        r#"#pragma code_page(65001)
1 ICON "{root}/icons/icon.ico"

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
      VALUE "FileDescription", "{description}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "{bin}"
      VALUE "LegalCopyright", "lulu-loopp"
      VALUE "OriginalFilename", "{bin}.exe"
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
    let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join(format!("{bin}.rc"));
    std::fs::write(&path, script).unwrap();
    embed_resource::compile_for(&path, [bin], embed_resource::NONE).manifest_optional().unwrap();
}
