use std::env;
use std::path::PathBuf;
use std::process::Command;

const LIB_DIR: &str = "/usr/lib64/sailbubbles-daemon";

// The HEIC converter needs the Sailfish sysroot and a libheif build
// (HEIF_PREFIX), so it is only built for the phone (SAILFISH_SYSROOT set);
// host builds get the `sailfish` cfg off.
fn main() {
    println!("cargo:rustc-check-cfg=cfg(sailfish)");
    println!("cargo:rerun-if-env-changed=SAILFISH_SYSROOT");
    println!("cargo:rerun-if-env-changed=HEIF_PREFIX");
    println!("cargo:rerun-if-changed=native/heic.cpp");
    let Ok(sysroot) = env::var("SAILFISH_SYSROOT") else { return };
    let heif = env::var("HEIF_PREFIX").expect("HEIF_PREFIX not set");

    let target = env::var("TARGET").unwrap().replace('-', "_");
    let cxx = env::var(format!("CXX_{target}")).expect("CXX_<target> not set");
    let ar = env::var(format!("AR_{target}")).expect("AR_<target> not set");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let inc = |d: &str| format!("-I{sysroot}/usr/include/{d}");
    let compile = |src: &str, obj: &PathBuf, extra: &[String]| {
        run(Command::new(&cxx)
            .args(["-std=c++11", "-fPIC", "-O2", "-Wall", "-Wextra", "-c", src, "-o"])
            .arg(obj)
            .args([inc("qt5"), inc("qt5/QtCore"), inc("qt5/QtGui")])
            .args(extra));
    };
    let heic = out.join("obd_heic.o");
    compile("native/heic.cpp", &heic, &[format!("-I{heif}/include")]);
    run(Command::new(&ar).arg("crs").arg(out.join("libobd_native.a")).arg(&heic));

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={heif}/lib");
    println!("cargo:rustc-link-lib=static=obd_native");
    for lib in ["heif", "Qt5Gui", "Qt5Core", "stdc++"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
    // libheif and libde265 ship inside the daemon package, not in Sailfish
    println!("cargo:rustc-link-arg=-Wl,-rpath,{LIB_DIR}");
    println!("cargo:rustc-link-arg=-Wl,-rpath-link,{heif}/lib");
    println!("cargo:rustc-cfg=sailfish");
}

fn run(cmd: &mut Command) {
    let status = cmd.status().expect("failed to start build tool");
    assert!(status.success(), "{cmd:?} failed");
}
