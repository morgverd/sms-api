
fn main() {
    feature_conflicts();

    let version = get_version();
    println!("cargo:rustc-env=VERSION={}", version);
    println!("cargo:warning=Feature tagged version: {}", version);

    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_HTTP_SERVER");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_SENTRY");
    println!("cargo:rerun-if-changed=build.rs");
}

fn feature_conflicts() {

    // TLS.
    let tls_rustls = cfg!(feature = "tls-rustls");
    let tls_native = cfg!(feature = "tls-native");

    if tls_rustls && tls_native {
        panic!("Cannot enable both 'tls-rustls' and 'tls-native' features simultaneously. Choose one.");
    }
    if !tls_rustls && !tls_native {
        println!("cargo:warning=No TLS backend selected. Consider enabling either 'tls-rustls' or 'tls-native' features for production use!");
    }

    // Database.
    if !cfg!(feature = "db-sqlite")
        // && !cfg!(feature = "db-postgres-rustls")
        // && !cfg!(feature = "db-postgres-native")
    {
        panic!("At least one database backend feature must be enabled!");
    }
}

fn get_version() -> String {
    let mut suffixes = Vec::new();
    if cfg!(feature = "http-server") {
        suffixes.push("http");
    }
    if cfg!(feature = "sentry") {
        suffixes.push("sentry");
    }

    let version = env!("CARGO_PKG_VERSION");
    let full_version = if suffixes.is_empty() {
        version.to_string()
    } else {
        format!("{}+{}", version, suffixes.join("."))
    };

    full_version
}