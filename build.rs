// Prevent feature conflicts

fn main() {

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
    if !cfg!(feature = "db-sqlite") &&
        !cfg!(feature = "db-postgres-rustls") &&
        !cfg!(feature = "db-postgres-native")
    {
        panic!("At least one database backend feature must be enabled!");
    }
}