//! Run the deterministic cleanup regression harness and print a summary.
//!
//! ```text
//! cargo run -p whimpr-core --example cleanup_eval
//! ```

fn main() {
    match whimpr_core::cleanup::run_eval() {
        Ok(report) => {
            println!(
                "cleanup eval: {}/{} passed",
                report.passed, report.total
            );
            if report.ok() {
                std::process::exit(0);
            }
            for failure in &report.failed {
                eprintln!(
                    "FAIL [{}] {}: {}",
                    failure.category, failure.id, failure.detail
                );
            }
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("cleanup eval error: {error}");
            std::process::exit(2);
        }
    }
}
