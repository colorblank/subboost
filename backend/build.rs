use std::{
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "map" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn collect_files(root: &Path, dir: &Path, entries: &mut Vec<(String, &'static str, PathBuf)>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, entries);
        } else if path.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
            if let Ok(rel) = path.strip_prefix(root) {
                let web_path = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
                let mime = content_type_for(&path);
                let abs = fs::canonicalize(&path).unwrap_or(path);
                entries.push((web_path, mime, abs));
            }
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=SUBBOOST_EMBED_FRONTEND_DIR");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let generated = out_dir.join("embedded_frontend.rs");

    let frontend_dir = match env::var("SUBBOOST_EMBED_FRONTEND_DIR") {
        Ok(val) if val.trim().is_empty() || val.trim().eq_ignore_ascii_case("none") => None,
        Ok(val) => Some(PathBuf::from(val)),
        Err(_) => Some(manifest_dir.join("../local/out")),
    };

    let mut entries = Vec::new();
    if let Some(dir) = frontend_dir {
        println!("cargo:rerun-if-changed={}", dir.display());
        if dir.join("index.html").is_file() {
            collect_files(&dir, &dir, &mut entries);
            entries.sort_by(|a, b| a.0.cmp(&b.0));
        }
    }

    let mut file = File::create(&generated).expect("create embedded_frontend.rs");
    writeln!(
        file,
        "pub const EMBEDDED_ASSETS: &[(&str, &str, &[u8])] = &["
    )
    .expect("write header");
    for (web_path, mime, abs_path) in &entries {
        let path_str = abs_path.to_string_lossy();
        writeln!(
            file,
            "    ({web_path:?}, {mime:?}, include_bytes!({path_str:?})),"
        )
        .expect("write asset entry");
    }
    writeln!(file, "];").expect("write footer");
}
