use crate::server::BackendWrapper;
use tower_lsp::{LspService, Server};
#[cfg(test)]
mod backend_testing;
mod common;
mod lsp;
mod server;
#[cfg(test)]
mod test;

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::build(|client| BackendWrapper::new(client)).finish();
    Server::new(stdin, stdout, socket).serve(service).await;
}
