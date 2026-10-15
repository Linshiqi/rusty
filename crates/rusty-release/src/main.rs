//! What the release workflow runs between the build and the upload, with
//! no Tauri CLI left to run it:
//!
//! ```text
//! rusty-release pack --stub rusty-setup.exe --dir <install dir> --out <installer>
//! rusty-release sign <file> --version <version>
//! rusty-release verify <file> --version <version>
//! ```
//!
//! `pack` appends the install directory to the installer
//! (`rusty_setup::payload`). `sign` writes `<file>.sig` as the Tauri CLI
//! did — base64 of a minisign signature — with the same key: the secret
//! half in `TAURI_SIGNING_PRIVATE_KEY` (base64 of minisign's secret key
//! file, as `cargo tauri signer generate` wrote it) and its password in
//! `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, or a file of that text named with
//! `--key`. The trusted comment carries the version, which the updater
//! holds the feed's to. `verify` is the updater's own check
//! (`rusty_embed::update::verify`), run before anything is published: a
//! signature the installed copies would refuse is a release that never
//! arrives, and nothing else would say so.

use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("pack") => pack(&args[1..]),
        Some("sign") => sign(&args[1..]),
        Some("verify") => verify(&args[1..]),
        _ => Err(
            "usage: rusty-release pack --stub <exe> --dir <dir> --out <exe>\n       \
                  rusty-release sign <file> --version <version> [--key <file>]"
                .to_string(),
        ),
    };
    if let Err(why) = result {
        eprintln!("rusty-release: {why}");
        std::process::exit(1);
    }
}

/// The value after `--name`.
fn option(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .cloned()
}

fn verify(args: &[String]) -> Result<(), String> {
    let file = args.first().ok_or("which file?")?;
    let version = option(args, "--version").ok_or("--version is missing")?;
    let data = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
    let sig = format!("{file}.sig");
    let signature = std::fs::read_to_string(&sig).map_err(|e| format!("{sig}: {e}"))?;
    rusty_embed::update::verify(&data, &signature, &version)?;
    println!("{file}: verifies for {version} against the key rusty carries");
    Ok(())
}

fn pack(args: &[String]) -> Result<(), String> {
    let need = |name: &str| option(args, name).ok_or(format!("{name} is missing"));
    let (stub, dir, out) = (need("--stub")?, need("--dir")?, need("--out")?);
    let stub = std::fs::read(&stub).map_err(|e| format!("{stub}: {e}"))?;
    let mut file =
        std::io::BufWriter::new(std::fs::File::create(&out).map_err(|e| format!("{out}: {e}"))?);
    let count = rusty_setup::payload::pack(&stub, Path::new(&dir), &mut file)
        .map_err(|e| format!("packing {dir}: {e}"))?;
    drop(file);
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    println!("{out}: {count} files, {:.1} MB", size as f64 / 1e6);
    Ok(())
}

fn sign(args: &[String]) -> Result<(), String> {
    let file = args
        .first()
        .filter(|arg| !arg.starts_with("--"))
        .map(PathBuf::from)
        .ok_or("which file?")?;
    let version = option(args, "--version").ok_or("--version is missing")?;
    let encoded = match option(args, "--key") {
        Some(path) => std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?,
        None => std::env::var("TAURI_SIGNING_PRIVATE_KEY")
            .map_err(|_| "no key: TAURI_SIGNING_PRIVATE_KEY is not set and --key names none")?,
    };
    let password = std::env::var("TAURI_SIGNING_PRIVATE_KEY_PASSWORD").unwrap_or_default();
    let key = secret_key(&encoded, password)?;
    let data = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let trusted = format!("timestamp:{now}\tfile:{name}\tversion:{version}");
    let signature = minisign::sign(None, &key, data.as_slice(), Some(&trusted), None)
        .map_err(|e| format!("signing: {e}"))?;
    let out = PathBuf::from(format!("{}.sig", file.display()));
    std::fs::write(&out, base64(&signature.to_string()))
        .map_err(|e| format!("{}: {e}", out.display()))?;
    println!("{}: signed for {version}", out.display());
    Ok(())
}

/// The secret key from its base64 text, decrypted with `password` — an
/// empty one included, which is what `signer generate` writes when none
/// is given.
fn secret_key(encoded: &str, password: String) -> Result<minisign::SecretKey, String> {
    use base64::Engine;
    let text = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| format!("the key is not base64: {e}"))?;
    let text = String::from_utf8(text).map_err(|_| "the key is not text")?;
    let boxed = minisign::SecretKeyBox::from_string(&text).map_err(|e| format!("the key: {e}"))?;
    boxed
        .into_secret_key(Some(password))
        .map_err(|e| format!("the key does not open with that password: {e}"))
}

fn base64(text: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(text)
}
