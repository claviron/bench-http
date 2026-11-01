#[cfg(target_os = "linux")]
mod cpu;
#[cfg(target_os = "linux")]
mod observe;

fn main() -> std::process::ExitCode {
    #[cfg(target_os = "linux")]
    match observe::execute() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Measurement failed: {error}");
            std::process::ExitCode::from(2)
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("brk-observe requires Linux process CPU counters");
        std::process::ExitCode::from(2)
    }
}
