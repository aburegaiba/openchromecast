use crate::crypto::Identity;
use anyhow::{Context, Result, ensure};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

const MAX_PEM_BYTES: u64 = 64 * 1024;

pub fn load(private_root: &Path) -> Result<Identity> {
    let owner = unsafe { libc::geteuid() };
    let root = fs::symlink_metadata(private_root).context("private Cast storage unavailable")?;
    check_directory(&root, owner)?;
    let directory = private_root.join("identity");
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Identity::generate(),
        Err(error) => return Err(error).context("private Cast identity unavailable"),
    };
    check_directory(&metadata, owner)?;
    // An existing but incomplete or invalid identity is never silently replaced.
    let certificate = read_private_pem(&directory.join("certificate.pem"), owner)?;
    let key = read_private_pem(&directory.join("key.pem"), owner)?;
    Identity::from_device_pem(&certificate, &key).context("configured Cast identity is invalid")
}

fn check_directory(metadata: &Metadata, owner: u32) -> Result<()> {
    ensure!(
        metadata.is_dir() && metadata.uid() == owner && metadata.mode() & 0o7777 == 0o700,
        "Cast identity directory must be private and owned by the application"
    );
    Ok(())
}

fn check_file(metadata: &Metadata, owner: u32) -> Result<()> {
    ensure!(
        metadata.is_file()
            && metadata.uid() == owner
            && metadata.mode() & 0o7777 == 0o600
            && metadata.nlink() == 1
            && (1..=MAX_PEM_BYTES).contains(&metadata.len()),
        "Cast identity file must be private, bounded and owned by the application"
    );
    Ok(())
}

fn read_private_pem(path: &Path, owner: u32) -> Result<String> {
    let expected = fs::symlink_metadata(path).context("Cast identity file unavailable")?;
    check_file(&expected, owner)?;
    let file: File = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .context("cannot open private Cast identity")?;
    let actual = file.metadata()?;
    check_file(&actual, owner)?;
    ensure!(
        expected.dev() == actual.dev() && expected.ino() == actual.ino(),
        "Cast identity file changed while opening"
    );
    let mut bytes = Vec::new();
    file.take(MAX_PEM_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_PEM_BYTES,
        "Cast identity file exceeds limit"
    );
    String::from_utf8(bytes).context("Cast identity PEM must be UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair};
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
    use std::sync::OnceLock;

    struct Fixture {
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("cast-identity-test-{}", uuid::Uuid::new_v4()));
            fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
            Self { root }
        }

        fn install(&self) {
            let directory = self.root.join("identity");
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
            let (certificate, key) = credentials();
            self.write("certificate.pem", certificate.as_bytes());
            self.write("key.pem", key.as_bytes());
        }

        fn write(&self, name: &str, bytes: &[u8]) {
            let path = self.root.join("identity").join(name);
            fs::write(&path, bytes).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Every path here is a fresh UUID-named test directory with fictive keys.
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    fn credentials() -> &'static (String, String) {
        static MATERIAL: OnceLock<(String, String)> = OnceLock::new();
        MATERIAL.get_or_init(|| {
            let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
            let pem = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
            let pair = KeyPair::from_pem(&pem).unwrap();
            let certificate = CertificateParams::new(vec!["cast-test.invalid".into()])
                .unwrap()
                .self_signed(&pair)
                .unwrap();
            (certificate.pem(), pem)
        })
    }

    #[test]
    fn no_identity_directory_keeps_the_existing_self_signed_default() {
        let fixture = Fixture::new();
        let identity = load(&fixture.root).unwrap();
        assert_eq!(identity.auth_cert_der(), identity.tls_cert_der());
        assert!(!fixture.root.join("identity").exists());
    }

    #[test]
    fn supplied_identity_has_separate_short_lived_tls_and_binds_nonce() {
        use rsa::pkcs8::DecodePrivateKey;
        use rsa::signature::Verifier;
        let fixture = Fixture::new();
        fixture.install();
        let identity = load(&fixture.root).unwrap();
        assert_ne!(identity.auth_cert_der(), identity.tls_cert_der());
        let key = rsa::RsaPrivateKey::from_pkcs8_pem(&credentials().1).unwrap();
        let verifier = rsa::pkcs1v15::VerifyingKey::<rsa::sha2::Sha256>::new(key.to_public_key());
        let mut signed = vec![0x31; 32];
        signed.extend_from_slice(identity.tls_cert_der());
        let signature =
            rsa::pkcs1v15::Signature::try_from(identity.sign(&signed).as_slice()).unwrap();
        verifier.verify(&signed, &signature).unwrap();
        signed[0] ^= 1;
        assert!(verifier.verify(&signed, &signature).is_err());
    }

    #[test]
    fn empty_partial_and_corrupt_configuration_fail_without_fallback() {
        let fixture = Fixture::new();
        fixture.install();
        fs::remove_file(fixture.root.join("identity/key.pem")).unwrap();
        assert!(load(&fixture.root).is_err());
        fixture.write("key.pem", b"not a key");
        assert!(load(&fixture.root).is_err());
        fixture.write("key.pem", credentials().1.as_bytes());
        fs::remove_file(fixture.root.join("identity/certificate.pem")).unwrap();
        assert!(load(&fixture.root).is_err());
        fixture.write("certificate.pem", b"not a certificate");
        assert!(load(&fixture.root).is_err());
        fs::remove_file(fixture.root.join("identity/certificate.pem")).unwrap();
        fs::remove_file(fixture.root.join("identity/key.pem")).unwrap();
        assert!(load(&fixture.root).is_err());
    }

    #[test]
    fn certificate_and_private_key_must_match() {
        let fixture = Fixture::new();
        fixture.install();
        let other = KeyPair::generate().unwrap();
        let certificate = CertificateParams::new(vec!["different-test.invalid".into()])
            .unwrap()
            .self_signed(&other)
            .unwrap();
        fixture.write("certificate.pem", certificate.pem().as_bytes());
        assert!(load(&fixture.root).is_err());
    }

    #[test]
    fn symlinked_root_identity_or_file_is_rejected() {
        let fixture = Fixture::new();
        fixture.install();
        let root_link = fixture.root.join("linked-root");
        symlink(&fixture.root, &root_link).unwrap();
        assert!(load(&root_link).is_err());
        fs::rename(fixture.root.join("identity"), fixture.root.join("original")).unwrap();
        symlink(fixture.root.join("original"), fixture.root.join("identity")).unwrap();
        assert!(load(&fixture.root).is_err());
        fs::remove_file(fixture.root.join("identity")).unwrap();
        fs::rename(fixture.root.join("original"), fixture.root.join("identity")).unwrap();
        fs::rename(
            fixture.root.join("identity/key.pem"),
            fixture.root.join("original-key"),
        )
        .unwrap();
        symlink(
            fixture.root.join("original-key"),
            fixture.root.join("identity/key.pem"),
        )
        .unwrap();
        assert!(load(&fixture.root).is_err());
    }

    #[test]
    fn public_permissions_wrong_owner_and_hard_links_are_rejected() {
        let fixture = Fixture::new();
        fixture.install();
        let directory = fixture.root.join("identity");
        let key = directory.join("key.pem");
        fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load(&fixture.root).is_err());
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(load(&fixture.root).is_err());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let metadata = fs::metadata(&key).unwrap();
        assert!(check_file(&metadata, metadata.uid() + 1).is_err());
        fs::hard_link(&key, fixture.root.join("extra-link")).unwrap();
        assert!(load(&fixture.root).is_err());
    }

    #[test]
    fn empty_oversized_and_non_utf8_files_are_rejected() {
        let fixture = Fixture::new();
        fixture.install();
        for bytes in [
            Vec::new(),
            vec![b'x'; MAX_PEM_BYTES as usize + 1],
            vec![0xff, 0xfe],
        ] {
            fixture.write("key.pem", &bytes);
            assert!(load(&fixture.root).is_err());
        }
    }
}
