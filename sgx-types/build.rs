use std::env;

// Kept free of sgx-build on purpose. sgx-build depends on cc, and a downstream
// crate that replaces crates-io `getrandom` with an SGX implementation then
// closes a dependency cycle: sgx-types -> sgx-build -> cc -> jobserver ->
// getrandom -> sgx-trts -> sgx-types. This mirrors SgxBuilder::get_sdk_lib_path.
fn sdk_lib_path() -> String {
    let sgx_sdk = env::var("SGX_SDK").unwrap_or_else(|_| "/opt/intel/sgxsdk".to_string());
    let sgx_arch = env::var("SGX_ARCH").unwrap_or_else(|_| {
        if cfg!(target_pointer_width = "32") {
            "x86".to_string()
        } else {
            "x64".to_string()
        }
    });
    let lib_dir = if sgx_arch == "x86" { "lib" } else { "lib64" };
    format!("{sgx_sdk}/{lib_dir}")
}

fn main() {
    // Set library search path for SGX SDK
    println!("cargo:rustc-link-search=native={}", sdk_lib_path());

    // Enable simulation feature based on SGX_MODE environment variable
    let sgx_mode = env::var("SGX_MODE").unwrap_or_default().to_uppercase();
    if sgx_mode == "SW" || sgx_mode == "SIM" {
        println!("cargo:rustc-cfg=feature=\"simulation\"");
    }
    println!("cargo:rustc-env=SGX_MODE={sgx_mode}");
}
