//! Reports the compiled GPU backend to the crate via `TG_GPU_BACKEND`,
//! so the GUI can tell users what their binary supports.

fn main() {
    let backend = if std::env::var_os("CARGO_FEATURE_CUDA").is_some() {
        "cuda"
    } else if std::env::var_os("CARGO_FEATURE_VULKAN").is_some() {
        "vulkan"
    } else {
        "cpu"
    };
    println!("cargo:rustc-env=TG_GPU_BACKEND={backend}");
}
