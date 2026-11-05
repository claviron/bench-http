//! Write the certificate store the TLS hosts read: one authority and one
//! server certificate, in the directory layout of a Claviron certificate
//! store. A native host reads the same PEM files directly.
//!
//! ```text
//! <dir>/certstore.yaml
//! <dir>/ca/bench/ca.crt
//! <dir>/servers/<name>/fullchain.pem
//! <dir>/servers/<name>/privkey.pem
//! ```

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use clap::Parser;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
};

#[derive(Parser)]
#[command(about = "Generate the benchmark certificate store")]
struct Args {
    /// Directory to create; an existing one is refused.
    #[arg(long)]
    dir: PathBuf,
    /// Host name the server certificate is issued for.
    #[arg(long)]
    name: String,
    /// Validity of the authority and the certificate, in days.
    #[arg(long)]
    days: u32,
}

fn validity(params: &mut CertificateParams, days: u32) {
    let now = std::time::SystemTime::now();
    params.not_before = now.into();
    params.not_after =
        (now + std::time::Duration::from_secs(u64::from(days) * 86_400)).into();
}

fn write(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    fs::create_dir_all(path.parent().expect("store paths have a parent"))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    if private {
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.days == 0 {
        return Err("--days must be positive".into());
    }
    if args.dir.exists() {
        return Err(format!("{} already exists", args.dir.display()).into());
    }

    let authority_key = KeyPair::generate()?;
    let mut authority = CertificateParams::new(Vec::<String>::new())?;
    let mut subject = DistinguishedName::new();
    subject.push(DnType::CommonName, "Benchmark CA");
    authority.distinguished_name = subject;
    authority.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    authority.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    validity(&mut authority, args.days);
    let authority_certificate = authority.self_signed(&authority_key)?;
    let issuer = Issuer::from_params(&authority, &authority_key);

    let key = KeyPair::generate()?;
    let mut server = CertificateParams::new(vec![args.name.clone()])?;
    let mut subject = DistinguishedName::new();
    subject.push(DnType::CommonName, args.name.as_str());
    server.distinguished_name = subject;
    server.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    server.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    validity(&mut server, args.days);
    let certificate = server.signed_by(&key, &issuer)?;

    let server_dir = args.dir.join("servers").join(&args.name);
    write(&args.dir.join("certstore.yaml"), b"dir: .\n", false)?;
    write(
        &args.dir.join("ca").join("bench").join("ca.crt"),
        authority_certificate.pem().as_bytes(),
        false,
    )?;
    write(
        &server_dir.join("fullchain.pem"),
        (certificate.pem() + &authority_certificate.pem()).as_bytes(),
        false,
    )?;
    write(&server_dir.join("privkey.pem"), key.serialize_pem().as_bytes(), true)?;
    println!("Certificate store: {}", args.dir.display());
    println!("Authority: {}", args.dir.join("ca/bench/ca.crt").display());
    Ok(())
}
