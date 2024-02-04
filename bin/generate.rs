use std::fs;
use std::path::Path;
use std::env;
use std::io::Write;

//  cargo run --bin new_project my_project_name

fn main() {
    // Assume the first argument is the project name
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <project_name>", args[0]);
        std::process::exit(1);
    }
    let project_name = &args[1];

    //NOTE: for testing purposes, we put our new test project
    //   code under output_logs so the folder is ignored by git

    // Create a new directory for the project
    let test_folder = Path::new("output_logs");
    let project_path = test_folder.join(&project_name);

    if project_path.exists() {
        eprintln!("Error: Project directory '{}' already exists.", project_name);
        std::process::exit(1);
    }
    fs::create_dir_all(project_path.join("src")).expect("Failed to create project directories.");

    // Create a basic Cargo.toml
    let mut cargo_toml = fs::File::create(project_path.join("Cargo.toml")).expect("Failed to create Cargo.toml");
    writeln!(cargo_toml, "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2018\"\n\n[dependencies]\nyour_crate = \"0.1.0\"", project_name).expect("Failed to write Cargo.toml");

    // Create a basic main.rs
    let mut main_rs = fs::File::create(project_path.join("src/main.rs")).expect("Failed to create main.rs");
    writeln!(main_rs, "fn main() {{\n    println!(\"Hello, world!\");\n}}").expect("Failed to write main.rs");

    println!("New project '{}' has been successfully created.", project_name);
}