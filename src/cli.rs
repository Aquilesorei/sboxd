use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "sbox",
    author = "Aquilesorei",
    version = "0.2.0",
    about = "Zero-config native Linux security sandbox for development commands",
    allow_external_subcommands = true
)]
pub struct Cli {
    #[arg(short = 'n', long = "offline", global = true)]
    pub offline: bool,

    #[arg(short = 'e', long = "allow-env", global = true)]
    pub allow_env: bool,

    #[arg(short = 'o', long = "allow-net-out", global = true)]
    pub allow_net_out: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
   
    Run {
   
        #[arg(short = 'n', long = "offline")]
        offline: bool,
        #[arg(short = 'e', long = "allow-env")]
        allow_env: bool,
   
        #[arg(short = 'o', long = "allow-net-out")]
        allow_net_out: bool,
        command: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    

    Lock,

    #[command(hide = true)]
    Shim {
        #[command(subcommand)]
        action: ShimAction,
    },
    
    
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Subcommand, Debug)]
pub enum ShimAction {
    Install,

    Verify,
}
