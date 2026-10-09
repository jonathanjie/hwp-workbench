use hwp_verify::worker::{run_worker, WorkerStatus};

fn main() -> std::process::ExitCode {
    match run_worker(std::io::stdin().lock(), std::io::stdout().lock()) {
        Ok(WorkerStatus::Success) => std::process::ExitCode::SUCCESS,
        Ok(WorkerStatus::Failure) | Err(_) => std::process::ExitCode::from(2),
    }
}
