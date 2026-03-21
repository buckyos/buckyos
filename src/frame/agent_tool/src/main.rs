use agent_tool::cli::run_process;

#[tokio::main]
async fn main() {
    let output = run_process().await;
    if !output.stdout.is_empty() {
        println!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprintln!("{}", output.stderr);
    }
    std::process::exit(output.exit_code);
}
