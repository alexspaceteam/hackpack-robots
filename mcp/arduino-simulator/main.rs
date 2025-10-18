use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "arduino-simulator")]
#[command(about = "Arduino simulator for testing MCP communication", long_about = None)]
struct Args {
}

fn main() {
    let _args = Args::parse();
    println!("Arduino simulator started");
}
