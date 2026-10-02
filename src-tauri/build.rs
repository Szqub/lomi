fn main() {
    for feature in [
        "NATIVE_SMOKE",
        "CHAT_PROBE",
        "ANDROID_PROBE",
        "REMOTE_PROBE",
        "MCP_PROBE",
    ] {
        let variable = format!("CARGO_FEATURE_{feature}");
        println!("cargo:rerun-if-env-changed={variable}");
        assert!(
            !(std::env::var_os(&variable).is_some()
                && std::env::var("PROFILE").as_deref() == Ok("release")),
            "Qualification feature {feature} must not enter a release build"
        );
    }
    println!("cargo:rerun-if-changed=../packages/ai-runtime");
    println!("cargo:rerun-if-changed=../src/chat/provider-presets.ts");
    println!("cargo:rerun-if-changed=../scripts/prepare-ai-runtime.mjs");
    let target = std::env::var("TARGET").expect("missing Cargo target");
    println!("cargo:rustc-env=LOMI_AI_TARGET={target}");
    let mut prepare = std::process::Command::new("node");
    prepare
        .arg("../scripts/prepare-ai-runtime.mjs")
        .env("TARGET", &target);
    if std::env::var_os("CARGO_FEATURE_CHAT_PROBE").is_some()
        || std::env::var_os("CARGO_FEATURE_MCP_PROBE").is_some()
    {
        prepare.arg("--fixture");
    }
    assert!(
        prepare
            .status()
            .expect("Install Node and run pnpm install before building")
            .success(),
        "AI runtime preparation failed"
    );

    println!("cargo:rerun-if-changed=../packages/remote-terminal-runtime");
    assert!(
        std::process::Command::new("node")
            .arg("../packages/remote-terminal-runtime/build.mjs")
            .status()
            .expect("Remote runtime build requires Node and pnpm install")
            .success(),
        "Remote terminal runtime preparation failed"
    );

    {
        println!("cargo:rerun-if-changed=android-proto/emulator_controller.proto");
        let mut config = tonic_prost_build::Config::new();
        config.protoc_executable(protoc_bin_vendored::protoc_bin_path().unwrap());
        config.bytes([".android.emulation.control.Image.image"]);
        tonic_prost_build::configure()
            .build_server(false)
            .compile_with_config(
                config,
                &["android-proto/emulator_controller.proto"],
                &["android-proto"],
            )
            .expect("failed to compile the pinned Android emulator protocol");
    }
    let mut attributes = tauri_build::Attributes::new().plugin(
        "browser",
        tauri_build::InlinedPlugin::new().commands(&["signal"]),
    );
    if target.ends_with("-windows-msvc") {
        // Native test executables do not receive Tauri's resource manifest.
        // Embed the same manifest for tests and the application through the linker.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::try_build(attributes).expect("failed to build Tauri permissions");
}
