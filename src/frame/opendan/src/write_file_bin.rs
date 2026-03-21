use opendan::agent_tools_cli::run_process;

#[tokio::main]
async fn main() {
    let output = run_process().await;
    println!("{}", output.stdout);
    std::process::exit(output.exit_code);
}
