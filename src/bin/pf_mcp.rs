//! `pf_mcp` — the PathForge MCP server on stdio. See `path_forge::mcp` for the tools.
//!
//! Relative scene and export paths resolve under $PATH_FORGE_HOME (default: Documents/PathForge).

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("pf_mcp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if std::env::args().any(|a| a == "--skill") {
        print!("{}", path_forge::skill::SKILL_MD);
        return;
    }
    if let Err(e) = path_forge::mcp::serve_stdio() {
        eprintln!("pf_mcp: {e}");
        std::process::exit(1);
    }
}
