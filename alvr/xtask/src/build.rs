use alvr_filesystem::{self as afs, Layout};
use std::{
    env,
    fmt::{self, Display, Formatter},
    fs,
    path::PathBuf,
    vec,
};
use xshell::{Shell, cmd};

fn copy_dir_contents(source: &std::path::Path, destination: &std::path::Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir_contents(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).unwrap();
        }
    }
}

fn set_vrx_model_poses(model: &mut serde_json::Value) {
    // The public client sends OpenXR grip poses unchanged for VRX. Therefore
    // openxr_grip is identity. The aim transform was measured with xrLocateSpace
    // on both PS0012P0 controllers, not borrowed from a different controller.
    let poses: serde_json::Value =
        serde_json::from_str(include_str!("../resources/vrx_controller_poses.json")).unwrap();
    let components = model["components"]
        .as_object_mut()
        .expect("VRX model components");
    for (name, pose) in poses.as_object().unwrap() {
        components.insert(name.clone(), pose.clone());
    }
}

fn sync_vrx_model_poses(resources: &std::path::Path) {
    for side in ["left", "right"] {
        let name = format!("alvr_vrx_controller_{side}");
        let path = resources
            .join("rendermodels")
            .join(&name)
            .join(format!("{name}.json"));
        // A model can be absent from custom builds that omit VRX resources.
        if !path.exists() {
            continue;
        }
        let mut model = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        set_vrx_model_poses(&mut model);
        fs::write(&path, serde_json::to_vec_pretty(&model).unwrap()).unwrap();
    }
}

pub fn sync_spaces_openxr_loader(source: PathBuf) {
    // Keep the VRX client on the public Snapdragon Spaces loader. The SDK is
    // stored outside this repository so the loader is not duplicated in Git.
    let destination = afs::deps_dir().join("android_openxr/arm64-v8a/libopenxr_loader_spaces.so");

    if !source.is_file() {
        panic!(
            "Snapdragon Spaces OpenXR loader not found at {}. Restore the official Spaces SDK \
            assets before building the VRX client.",
            source.display()
        );
    }
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    if !destination.is_file() || fs::read(&source).unwrap() != fs::read(&destination).unwrap() {
        fs::copy(&source, &destination).unwrap();
    }
}

#[cfg(test)]
mod vrx_input_tests {
    use serde_json::Value;
    use std::path::Path;

    #[test]
    fn vrx_pose_installation_preserves_meshes_and_animations() {
        let mut model = serde_json::json!({"thumbnail": "local.png", "components": {
            "body": {"filename": "body.obj", "component_local": {"origin": [0, 0, 0]}},
            "trigger": {"filename": "trigger.obj", "motion": {"trigger_path": "/input/trigger"}},
            "tip": {"component_local": {"origin": [0, 0, 0]}},
            "handgrip": {"component_local": {"origin": [0, 0, 0.025]}}
        }});
        let original = model.clone();
        super::set_vrx_model_poses(&mut model);
        assert_eq!(model["components"]["body"], original["components"]["body"]);
        assert_eq!(
            model["components"]["trigger"],
            original["components"]["trigger"]
        );
        assert_eq!(
            model["components"]["handgrip"],
            original["components"]["handgrip"]
        );
        assert_eq!(model["thumbnail"], original["thumbnail"]);
        assert_eq!(
            model["components"]["tip"],
            original["components"]["tip"]
        );
        assert_eq!(
            model["components"]["openxr_grip"]["component_local"]["origin"],
            serde_json::json!([0.0, 0.0, 0.0])
        );
        let once = model.clone();
        super::set_vrx_model_poses(&mut model);
        assert_eq!(model, once);
    }

    #[test]
    fn native_vrx_bindings_never_request_device_simulation() {
        let profile: Value = serde_json::from_str(include_str!(
            "../resources/vrx/input/alvr_vrx_controller_profile.json"
        ))
        .unwrap();
        let remapping: Value = serde_json::from_str(include_str!(
            "../resources/vrx/input/alvr_vrx_controller_remapping.json"
        ))
        .unwrap();
        assert_eq!(profile["controller_type"], "alvr_vrx_controller");
        assert!(profile.get("compatibility_mode_controller_type").is_none());
        assert_eq!(remapping["to_controller_type"], profile["controller_type"]);
        for layout in remapping["layouts"].as_array().unwrap() {
            for key in [
                "simulate_controller_type",
                "simulate_render_model",
                "simulate_HMD",
            ] {
                assert_eq!(layout[key], false, "{key} must be explicitly disabled");
            }
            // Every non-deleted destination must exist in the native profile,
            // including the mirrored pose and button paths.
            for entry in layout["autoremappings"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(layout["remappings"].as_array().into_iter().flatten())
            {
                if entry["remapping_mode"] == "delete" {
                    continue;
                }
                let to = entry["to"]
                    .as_str()
                    .or_else(|| entry["to"]["path"].as_str())
                    .unwrap();
                let component = to.strip_prefix("/user/hand/right").unwrap();
                assert!(profile["input_source"].get(component).is_some(), "{to}");
            }
        }
    }

    #[test]
    fn native_vrx_binding_editor_resources_are_complete() {
        let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/vrx");
        let profile: Value = serde_json::from_str(include_str!(
            "../resources/vrx/input/alvr_vrx_controller_profile.json"
        ))
        .unwrap();
        for side in ["left", "right"] {
            let image = &profile[format!("input_bindingui_{side}")];
            assert!(image["transform"].is_string());
            let path = image["image"]
                .as_str()
                .unwrap()
                .strip_prefix("{alvr_server}/")
                .unwrap();
            assert!(resources.join(path).is_file());
        }
        for (path, component) in profile["input_source"].as_object().unwrap() {
            let point = component["binding_image_point"].as_array().unwrap();
            assert_eq!(point.len(), 2, "{path}");
            assert!(point.iter().all(Value::is_number), "{path}");
        }
    }

    #[test]
    fn vrchat_native_binding_uses_valid_vrx_inputs_and_calibrated_aim() {
        let profile: Value = serde_json::from_str(include_str!(
            "../resources/vrx/input/alvr_vrx_controller_profile.json"
        ))
        .unwrap();
        let binding: Value = serde_json::from_str(include_str!(
            "../resources/vrx/input/bindings/steam.app.438100_alvr_vrx_controller.json"
        ))
        .unwrap();
        assert_eq!(binding["controller_type"], "alvr_vrx_controller");
        assert_eq!(binding["options"], serde_json::json!({}));
        for set in binding["bindings"].as_object().unwrap().values() {
            for kind in ["sources", "poses", "skeleton", "haptics"] {
                for entry in set[kind].as_array().into_iter().flatten() {
                    let path = entry["path"].as_str().unwrap();
                    let path = path
                        .strip_prefix("/user/hand/left")
                        .or_else(|| path.strip_prefix("/user/hand/right"))
                        .unwrap();
                    assert!(profile["input_source"].get(path).is_some(), "{path}");
                }
            }
        }
        let poses = binding["bindings"]["/actions/global"]["poses"]
            .as_array()
            .unwrap();
        assert_eq!(poses.len(), 2);
        for pose in poses {
            assert!(pose["path"].as_str().unwrap().ends_with("/pose/openxr_aim"));
        }
    }
}

#[derive(Clone, Copy)]
pub enum Profile {
    Debug,
    Release,
    Distribution,
}

impl Display for Profile {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let string = match self {
            Profile::Distribution => "distribution",
            Profile::Release => "release",
            Profile::Debug => "debug",
        };
        write!(f, "{string}")
    }
}

pub fn build_server_lib(profile: Profile, root: Option<String>, reproducible: bool) {
    let sh = Shell::new().unwrap();

    let mut flags = vec![];
    match profile {
        Profile::Distribution => {
            flags.push("--profile");
            flags.push("distribution");
        }
        Profile::Release => flags.push("--release"),
        Profile::Debug => (),
    }
    if reproducible {
        flags.push("--locked");
    }
    let flags_ref = &flags;

    let artifacts_dir = afs::target_dir().join(profile.to_string());

    let build_dir = afs::build_dir().join("alvr_server_core");
    sh.create_dir(&build_dir).unwrap();

    if let Some(root) = root {
        sh.set_var("ALVR_ROOT_DIR", root);
    }

    let _push_guard = sh.push_dir(afs::crate_dir("server_core"));
    cmd!(sh, "cargo build {flags_ref...}").run().unwrap();

    sh.copy_file(
        artifacts_dir.join(afs::dynlib_fname("alvr_server_core")),
        &build_dir,
    )
    .unwrap();

    if cfg!(windows) {
        sh.copy_file(artifacts_dir.join("alvr_server_core.pdb"), &build_dir)
            .unwrap();
    }

    let out = build_dir.join("alvr_server_core.h");
    cmd!(sh, "cbindgen --output {out}").run().unwrap();
}

pub fn build_streamer(
    profile: Profile,
    gpl: bool,
    root: Option<String>,
    reproducible: bool,
    profiling: bool,
    keep_config: bool,
) {
    let sh = Shell::new().unwrap();

    let build_layout = Layout::new(&afs::streamer_build_dir());

    let mut common_flags = vec![];
    match profile {
        Profile::Distribution => {
            common_flags.push("--profile");
            common_flags.push("distribution");
        }
        Profile::Release => common_flags.push("--release"),
        Profile::Debug => (),
    }
    if reproducible {
        common_flags.push("--locked");
    }

    let artifacts_dir = if cfg!(all(windows, target_arch = "aarch64")) {
        // Fix for cross compilation
        const TARGET: &str = "x86_64-pc-windows-msvc";

        common_flags.push("--target");
        common_flags.push(TARGET);

        afs::target_dir().join(TARGET).join(profile.to_string())
    } else {
        afs::target_dir().join(profile.to_string())
    };

    let common_flags_ref = &common_flags;

    let maybe_config = if keep_config {
        fs::read_to_string(build_layout.session()).ok()
    } else {
        None
    };

    sh.remove_path(afs::streamer_build_dir()).ok();
    sh.create_dir(build_layout.openvr_driver_lib_dir()).unwrap();
    sh.create_dir(&build_layout.executables_dir).unwrap();

    if let Some(config) = maybe_config {
        fs::write(build_layout.session(), config).ok();
    }

    if let Some(root) = root {
        sh.set_var("ALVR_ROOT_DIR", root);
    }

    // build server
    {
        let gpl_flag = if gpl {
            vec!["--features", "gpl"]
        } else {
            vec![]
        };

        let profiling_flag = if profiling {
            vec!["--features", "alvr_server_core/trace-performance"]
        } else {
            vec![]
        };

        let _push_guard = sh.push_dir(afs::crate_dir("server_openvr"));
        cmd!(
            sh,
            "cargo build {common_flags_ref...} {gpl_flag...} {profiling_flag...}"
        )
        .run()
        .unwrap();

        sh.copy_file(
            artifacts_dir.join(afs::dynlib_fname("alvr_server_openvr")),
            build_layout.openvr_driver_lib(),
        )
        .unwrap();

        if cfg!(windows) {
            sh.copy_file(
                artifacts_dir.join("alvr_server_openvr.pdb"),
                build_layout
                    .openvr_driver_lib_dir()
                    .join("alvr_server_openvr.pdb"),
            )
            .unwrap();
        }
    }

    // Build dashboard
    {
        let _push_guard = sh.push_dir(afs::crate_dir("dashboard"));
        cmd!(sh, "cargo build {common_flags_ref...}").run().unwrap();

        sh.copy_file(
            artifacts_dir.join(afs::exec_fname("alvr_dashboard")),
            build_layout.dashboard_exe(),
        )
        .unwrap();
    }

    // copy dependencies
    if cfg!(windows) {
        sh.copy_file(
            afs::workspace_dir().join("openvr/bin/win64/openvr_api.dll"),
            build_layout.openvr_driver_lib_dir(),
        )
        .unwrap();

        // copy ffmpeg binaries
        if gpl {
            let bin_dir = &build_layout.openvr_driver_lib_dir();
            sh.create_dir(bin_dir).unwrap();
            for lib_path in sh
                .read_dir(afs::deps_dir().join("windows/ffmpeg/bin"))
                .unwrap()
                .into_iter()
                .filter(|path| path.file_name().unwrap().to_string_lossy().contains(".dll"))
            {
                sh.copy_file(lib_path.clone(), bin_dir).unwrap();
            }
        }

        // copy libvpl.dll
        sh.copy_file(
            afs::deps_dir().join("windows/libvpl/alvr_build/bin/libvpl.dll"),
            build_layout.openvr_driver_lib_dir(),
        )
        .unwrap();
    } else if cfg!(target_os = "linux") {
        let firewall_script = afs::crate_dir("xtask").join("firewall/alvr_fw_config.sh");
        let firewalld = afs::crate_dir("xtask").join("firewall/alvr-firewalld.xml");
        let ufw = afs::crate_dir("xtask").join("firewall/ufw-alvr");

        // copy linux specific firewalls
        sh.create_dir(build_layout.firewall_script()).unwrap();
        sh.copy_file(firewall_script, build_layout.firewall_script())
            .unwrap();
        sh.copy_file(firewalld, build_layout.firewalld_config())
            .unwrap();
        sh.copy_file(ufw, build_layout.ufw_config()).unwrap();
    }

    // copy static resources
    {
        // copy driver manifest
        sh.copy_file(
            afs::crate_dir("xtask").join("resources/driver.vrdrivermanifest"),
            build_layout.openvr_driver_manifest(),
        )
        .unwrap();

        // Keep compatibility with existing local controller asset bundles.
        let private_vrx_resources = afs::workspace_dir().join("private/vrx-controller/resources");
        if private_vrx_resources.is_dir() {
            // SteamVR resolves {alvr_server} against the driver-root resources.
            copy_dir_contents(&private_vrx_resources, &build_layout.resources_dir());
            // Also provide the explicitly requested local win64 resource bundle.
            copy_dir_contents(
                &private_vrx_resources,
                &build_layout.openvr_driver_lib_dir().join("resources"),
            );
        }

        // Copy the public VRX model and ALVR input profiles last so an older
        // local bundle cannot replace the current assets or native mappings.
        let vrx_input_resources = afs::crate_dir("xtask").join("resources/vrx");
        copy_dir_contents(&vrx_input_resources, &build_layout.resources_dir());
        copy_dir_contents(
            &vrx_input_resources,
            &build_layout.openvr_driver_lib_dir().join("resources"),
        );
        sync_vrx_model_poses(&build_layout.resources_dir());
        sync_vrx_model_poses(&build_layout.openvr_driver_lib_dir().join("resources"));
    }
}

pub fn build_launcher(profile: Profile, reproducible: bool) {
    let sh = Shell::new().unwrap();

    let mut common_flags = vec![];
    match profile {
        Profile::Distribution => {
            common_flags.push("--profile");
            common_flags.push("distribution");
        }
        Profile::Release => common_flags.push("--release"),
        Profile::Debug => (),
    }
    if reproducible {
        common_flags.push("--locked");
    }
    let common_flags_ref = &common_flags;

    sh.create_dir(afs::launcher_build_dir()).unwrap();

    cmd!(sh, "cargo build -p alvr_launcher {common_flags_ref...}")
        .run()
        .unwrap();

    sh.copy_file(
        afs::target_dir()
            .join(profile.to_string())
            .join(afs::exec_fname("alvr_launcher")),
        afs::launcher_build_exe_path(),
    )
    .unwrap();
}

fn build_android_lib_impl(dir_name: &str, profile: Profile, link_stdcpp: bool, all_targets: bool) {
    let sh = Shell::new().unwrap();

    let mut ndk_flags = vec!["--no-strip", "-p", "28", "-t", "arm64-v8a"];

    if all_targets {
        ndk_flags.extend(["-t", "armeabi-v7a", "-t", "x86_64", "-t", "x86"]);
    }

    let mut rust_flags = vec![];
    match profile {
        Profile::Distribution => {
            rust_flags.push("--profile");
            rust_flags.push("distribution")
        }
        Profile::Release => rust_flags.push("--release"),
        Profile::Debug => (),
    }
    if !link_stdcpp {
        rust_flags.push("--no-default-features");
    }
    let rust_flags_ref = &rust_flags;

    let build_dir = afs::build_dir().join(format!("alvr_{dir_name}"));
    sh.create_dir(&build_dir).unwrap();

    let _push_guard = sh.push_dir(afs::crate_dir(dir_name));
    cmd!(
        sh,
        "cargo ndk {ndk_flags...} -o {build_dir} build {rust_flags_ref...}"
    )
    .run()
    .unwrap();

    let out = build_dir.join(format!("alvr_{dir_name}.h"));
    cmd!(sh, "cbindgen --output {out}").run().unwrap();
}

pub fn build_android_client_core_lib(profile: Profile, link_stdcpp: bool, all_targets: bool) {
    build_android_lib_impl("client_core", profile, link_stdcpp, all_targets)
}

pub fn build_android_client_openxr_lib(profile: Profile, link_stdcpp: bool) {
    build_android_lib_impl("client_openxr", profile, link_stdcpp, false)
}

pub fn build_android_client(profile: Profile) {
    let sh = Shell::new().unwrap();

    let mut flags = vec![];
    match profile {
        Profile::Distribution => {
            flags.push("--profile");
            flags.push("distribution")
        }
        Profile::Release => flags.push("--release"),
        Profile::Debug => (),
    }
    let flags_ref = &flags;

    const ARTIFACT_NAME: &str = "alvr_client_android";

    let target_dir = afs::target_dir();
    let build_dir = afs::build_dir().join(ARTIFACT_NAME);
    sh.create_dir(&build_dir).unwrap();

    // Create debug keystore (signing will be overwritten by CI)
    if env::var(format!(
        "CARGO_APK_{}_KEYSTORE",
        profile.to_string().to_uppercase()
    ))
    .is_err()
        && matches!(profile, Profile::Release | Profile::Distribution)
    {
        let keystore_path = build_dir.join("debug.keystore");
        if !keystore_path.exists() {
            let keytool = PathBuf::from(env::var("JAVA_HOME").expect("Env var JAVA_HOME not set"))
                .join("bin")
                .join(afs::exec_fname("keytool"));
            let pass = "alvrclient";

            let other = vec![
                "-genkey",
                "-v",
                "-alias",
                "androiddebugkey",
                "-dname",
                "CN=Android Debug,O=Android,C=US",
                "-keyalg",
                "RSA",
                "-keysize",
                "2048",
                "-validity",
                "10000",
            ];

            cmd!(
                sh,
                "{keytool} -keystore {keystore_path} -storepass {pass} -keypass {pass} {other...}"
            )
            .run()
            .unwrap();
        }
    }

    let _push_guard = sh.push_dir(afs::crate_dir("client_openxr"));
    cmd!(
        sh,
        "cargo apk build --target-dir={target_dir} {flags_ref...}"
    )
    .run()
    .unwrap();

    sh.copy_file(
        afs::target_dir()
            .join(profile.to_string())
            .join("apk/alvr_client_openxr.apk"),
        build_dir.join(format!("{ARTIFACT_NAME}.apk")),
    )
    .unwrap();
}
