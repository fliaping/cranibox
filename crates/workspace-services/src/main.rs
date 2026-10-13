fn main() {
    match agent_workspace_services::run(std::env::args().skip(1).collect()) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("workspace-services: {error:#}");
            std::process::exit(1);
        }
    }
}
