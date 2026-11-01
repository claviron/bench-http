#[cfg(any(target_os = "linux", all(test, unix)))]
mod cpu;
#[cfg(any(target_os = "linux", all(test, unix)))]
mod observe;
#[cfg(any(target_os = "linux", test))]
mod outcome;

fn main() -> std::process::ExitCode {
    #[cfg(any(target_os = "linux", all(test, unix)))]
    match observe::execute() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Measurement failed: {error}");
            std::process::ExitCode::from(2)
        }
    }

    #[cfg(not(any(target_os = "linux", all(test, unix))))]
    {
        eprintln!("brk-observe requires Linux process CPU counters");
        std::process::ExitCode::from(2)
    }
}
