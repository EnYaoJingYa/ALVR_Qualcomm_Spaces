use std::{env, error::Error, fs, path::PathBuf, process::Command};

fn run(command: &mut Command) -> Result<(), Box<dyn Error>> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Java bridge build failed: {status}").into())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=java/VrxProbeContext.java");
    println!("cargo:rerun-if-env-changed=ANDROID_HOME");
    println!("cargo:rerun-if-env-changed=JAVA_HOME");
    if env::var("CARGO_CFG_TARGET_OS")?.as_str() != "android" {
        return Ok(());
    }
    let sdk = PathBuf::from(env::var("ANDROID_HOME")?);
    let java = PathBuf::from(env::var("JAVA_HOME")?).join("bin");
    let executable = |name: &str| {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        }
    };
    let out = PathBuf::from(env::var("OUT_DIR")?);
    let classes = out.join("probe-classes");
    fs::create_dir_all(&classes)?;
    let android = sdk.join("platforms/android-32/android.jar");
    run(Command::new(java.join(executable("javac")))
        .args(["--release", "8", "-classpath"])
        .arg(&android)
        .arg("-d")
        .arg(&classes)
        .arg("java/VrxProbeContext.java"))?;
    run(Command::new(java.join(executable("java")))
        .arg("-cp")
        .arg(sdk.join("build-tools/34.0.0/lib/d8.jar"))
        .args(["com.android.tools.r8.D8", "--min-api", "28", "--lib"])
        .arg(android)
        .arg("--output")
        .arg(&out)
        .arg(classes.join("alvr/client/VrxProbeContext.class")))?;
    Ok(())
}
