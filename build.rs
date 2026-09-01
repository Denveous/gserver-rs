use std::env;
#[cfg(not(unix))]
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // The bundled NPC runtime exposes SQLite through its small FFI layer.  In
    // the execution image only the ABI-versioned runtime object is installed,
    // so pass that object through for binaries and test harnesses alike.
    let candidates = [
        "/lib/x86_64-linux-gnu/libsqlite3.so.0",
        "/usr/lib/x86_64-linux-gnu/libsqlite3.so.0",
        "/lib/aarch64-linux-gnu/libsqlite3.so.0",
        "/usr/lib/aarch64-linux-gnu/libsqlite3.so.0",
        "/lib64/libsqlite3.so.0",
        "/usr/lib64/libsqlite3.so.0",
    ];
    if let Some(path) = candidates.iter().find(|path| Path::new(path).exists()) {
        // Give rustc a conventional unversioned name in the private build
        // directory. `rustc-link-lib` is propagated to test harnesses, while
        // a raw link argument is only propagated to final binaries on older
        // Cargo versions.
        let out_dir = env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo");
        let link_dir = Path::new(&out_dir);
        let link_name = link_dir.join("libsqlite3.so");
        if !link_name.exists() {
            #[cfg(unix)]
            {
                let _ = std::os::unix::fs::symlink(path, &link_name);
            }
            #[cfg(not(unix))]
            {
                let _ = fs::copy(path, &link_name);
            }
        }
        println!("cargo:rustc-link-search=native={}", link_dir.display());
        println!("cargo:rustc-link-lib=dylib=sqlite3");
    } else {
        println!("cargo:rustc-link-lib=sqlite3");
    }

    // The bundled NPC runtime exposes HTTPS script requests through the
    // platform libcurl ABI.  Link the versioned runtime explicitly as well as
    // SQLite because link arguments from a path dependency are not propagated
    // to this crate's final binaries and test harnesses.
    let curl_candidates = [
        "/lib/x86_64-linux-gnu/libcurl.so.4",
        "/usr/lib/x86_64-linux-gnu/libcurl.so.4",
        "/lib/aarch64-linux-gnu/libcurl.so.4",
        "/usr/lib/aarch64-linux-gnu/libcurl.so.4",
        "/lib64/libcurl.so.4",
        "/usr/lib64/libcurl.so.4",
    ];
    if let Some(path) = curl_candidates.iter().find(|path| Path::new(path).exists()) {
        println!("cargo:rustc-link-arg=-Wl,{path}");
    } else {
        println!("cargo:rustc-link-lib=curl");
    }
}
