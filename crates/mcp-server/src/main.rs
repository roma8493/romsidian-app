//! Punto de entrada CLI del servidor MCP nativo `romsidian-mcp`.
//!
//! Uso:
//!   `romsidian-mcp --vault <ruta-a-la-boveda>`
//!   o definiendo la variable de entorno `ROMSIDIAN_VAULT`.

use std::env;
use std::io;
use std::process;

use romsidian_mcp::{resolve_vault_path, McpServer};

fn main() {
    let raw_args: Vec<String> = env::args().skip(1).collect();
    for arg in &raw_args {
        match arg.as_str() {
            "--help" | "-h" => {
                print_usage();
                return;
            }
            "--version" | "-V" => {
                println!("romsidian-mcp {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => {}
        }
    }

    let cwd = env::current_dir().unwrap_or_else(|err| {
        eprintln!("Error obteniendo directorio actual: {err}");
        process::exit(1);
    });
    let env_vault = env::var("ROMSIDIAN_VAULT").ok();
    let vault_path = resolve_vault_path(&raw_args, env_vault.as_deref(), &cwd);

    let mut server = match McpServer::open(&vault_path) {
        Ok(srv) => srv,
        Err(err) => {
            eprintln!(
                "No se pudo abrir la bóveda ROMSIDIAN en '{}': {err}",
                vault_path.display()
            );
            process::exit(1);
        }
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = server.process_stdio_stream(stdin.lock(), stdout.lock()) {
        eprintln!("Error en flujo stdio de romsidian-mcp: {err}");
        process::exit(1);
    }
}

fn print_usage() {
    println!(
        "romsidian-mcp {} - Servidor Model Context Protocol (MCP) sobre stdio para ROMSIDIAN\n\n\
USO:\n  \
  romsidian-mcp [--vault <RUTA>]\n\n\
OPCIONES:\n  \
  -v, --vault <RUTA>   Ruta a la carpeta raíz de la bóveda (por defecto: $ROMSIDIAN_VAULT o ./test-vault)\n  \
  -h, --help           Muestra esta ayuda\n  \
  -V, --version        Muestra la versión del servidor MCP",
        env!("CARGO_PKG_VERSION")
    );
}
