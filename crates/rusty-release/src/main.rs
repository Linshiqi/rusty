//! What the release workflow runs between the build and the upload:
//!
//! ```text
//! rusty-release pack --stub rusty-setup.exe --dir <install dir> --out <installer>
//! rusty-release sign <file> --version <version> [--key <file>]
//! rusty-release verify <file> --version <version>
//! ```
//!
//! `pack` appends the install directory to the installer
//! (`rusty_setup::payload`). `sign` writes `<file>.minisig`, an ordinary
//! minisign signature, with the updater key: minisign's secret key file,
//! as text or as base64 of it, in `RUSTY_UPDATE_KEY` or a file named with
//! `--key`, and its password in `RUSTY_UPDATE_KEY_PASSWORD` (none when
//! unset). The trusted comment carries the version, which the updater
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
                  rusty-release sign <file> --version <version> [--key <file>]\n       \
                  rusty-release verify <file> --version <version>"
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

/// Where `sign` puts a file's signature.
fn signature_of(file: &Path) -> PathBuf {
    PathBuf::from(format!("{}.minisig", file.display()))
}

fn verify(args: &[String]) -> Result<(), String> {
    let file = PathBuf::from(args.first().ok_or("which file?")?);
    let version = option(args, "--version").ok_or("--version is missing")?;
    let data = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let sig = signature_of(&file);
    let signature = std::fs::read_to_string(&sig).map_err(|e| format!("{}: {e}", sig.display()))?;
    rusty_embed::update::verify(&data, &signature, &version)?;
    println!(
        "{}: verifies for {version} against the key rusty carries",
        file.display()
    );
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
    let text = match option(args, "--key") {
        Some(path) => std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?,
        None => std::env::var("RUSTY_UPDATE_KEY")
            .map_err(|_| "no key: RUSTY_UPDATE_KEY is not set and --key names none")?,
    };
    let password = std::env::var("RUSTY_UPDATE_KEY_PASSWORD").unwrap_or_default();
    let key = secret_key(&text, password)?;
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
    let out = signature_of(&file);
    std::fs::write(&out, signature.to_string()).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("{}: signed for {version}", out.display());
    Ok(())
}

/// The secret key, decrypted with `password` — an empty one included,
/// which is what a key made without one opens with. The key file's text
/// is taken as it is, or base64 of it: one line is what a CI secret holds
/// most easily.
fn secret_key(text: &str, password: String) -> Result<minisign::SecretKey, String> {
    use base64::Engine;
    let text = text.trim();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(text)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let text = decoded.as_deref().unwrap_or(text);
    let boxed = minisign::SecretKeyBox::from_string(text).map_err(|e| format!("the key: {e}"))?;
    boxed
        .into_secret_key(Some(password))
        .map_err(|e| format!("the key does not open with that password: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key as minisign writes it and as base64 of that both open.
    #[test]
    fn a_key_signs_what_the_updater_checks() {
        use base64::Engine;
        let pair = minisign::KeyPair::generate_encrypted_keypair(Some(String::new())).unwrap();
        let text = pair.sk.to_box(None).unwrap().to_string();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&text);
        for form in [&text, &b64] {
            assert!(secret_key(form, String::new()).is_ok(), "{form}");
        }
        assert!(secret_key("not a key", String::new()).is_err());
    }
}
