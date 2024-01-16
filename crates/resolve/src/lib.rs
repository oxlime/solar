#![doc(
    html_logo_url = "https://raw.githubusercontent.com/paradigmxyz/sulk/main/assets/logo.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/paradigmxyz/sulk/main/assets/favicon.ico"
)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use std::path::{Path, PathBuf};
use sulk_ast::ast;
use sulk_data_structures::sync::Lrc;
use sulk_interface::{
    diagnostics::DiagCtxt,
    source_map::{FileName, FileResolver, ResolveError, SourceFile},
    sym, Result, Span,
};
use sulk_parse::{ParseSess, Parser};

pub struct Resolver<'a> {
    pub file_resolver: FileResolver<'a>,
    pub parse_sess: &'a ParseSess,
    files: Vec<Lrc<SourceFile>>,
}

impl<'a> Resolver<'a> {
    /// Creates a new resolver.
    pub fn new(parse_sess: &'a ParseSess) -> Self {
        Self {
            file_resolver: FileResolver::new(parse_sess.source_map()),
            parse_sess,
            files: Vec::new(),
        }
    }

    /// Returns the diagnostic context.
    pub fn dcx(&self) -> &'a DiagCtxt {
        &self.parse_sess.dcx
    }

    pub fn parse_and_resolve(
        &mut self,
        yul: bool,
        stdin: bool,
        paths: impl IntoIterator<Item = impl AsRef<Path>>,
    ) -> Result<()> {
        let dcx = self.dcx();
        let emit_resolve_error = |e: ResolveError| dcx.err(e.to_string()).emit();
        if stdin {
            let file = self.file_resolver.load_stdin().map_err(emit_resolve_error)?;
            self.resolve_file(yul, file)?;
        }
        for path in paths {
            let path = path.as_ref();
            let path = match path.canonicalize() {
                Ok(path) => {
                    match path.strip_prefix(std::env::current_dir().unwrap_or(PathBuf::from(""))) {
                        Ok(path) => path.to_path_buf(),
                        Err(_) => path,
                    }
                }
                Err(_) => path.to_path_buf(),
            };
            let file = self.file_resolver.resolve_file(&path, None).map_err(emit_resolve_error)?;
            self.resolve_file(yul, file)?;
        }
        Ok(())
    }

    fn resolve_file(&mut self, yul: bool, file: Lrc<SourceFile>) -> Result<()> {
        if self.files.iter().any(|f| Lrc::ptr_eq(f, &file)) {
            debug!("skipping file {}", file.name.display());
            return Ok(());
        }
        self.files.push(file.clone());

        let mut parser = Parser::from_source_file(self.parse_sess, &file);

        if yul {
            let file = parser.parse_yul_file_object().map_err(|e| e.emit())?;
            // TODO
            let _ = file;
            return Ok(());
        }

        debug!("parsing file {}", file.name.display());
        let source_unit = parser.parse_file().map_err(|e| e.emit())?;

        let parent = match &file.name {
            FileName::Real(path) => Some(path.as_path()),
            _ => None,
        };
        for item in &source_unit.items {
            match &item.kind {
                ast::ItemKind::Pragma(pragma) => {
                    self.check_pragma(item.span, pragma);
                }
                ast::ItemKind::Import(import) => {
                    // TODO: Unescape
                    let path_str = import.path.value.as_str();
                    let path = Path::new(path_str);
                    let file = self
                        .file_resolver
                        .resolve_file(path, parent)
                        .map_err(|e| self.dcx().err(e.to_string()).span(item.span).emit())?;
                    self.resolve_file(yul, file)?;
                }
                _ => {}
            }
        }

        // TODO: Rest

        Ok(())
    }

    fn check_pragma(&mut self, span: Span, pragma: &ast::PragmaDirective) {
        match &pragma.tokens {
            ast::PragmaTokens::Version(name, _version) => {
                if name.name != sym::solidity {
                    self.dcx()
                        .err("only `solidity` is supported as a version pragma")
                        .span(name.span)
                        .emit();
                }
                // TODO: Check version
            }
            ast::PragmaTokens::Custom(name, value) => {
                let name = name.value();
                let value = value.as_ref().map(ast::IdentOrStrLit::value);
                match (name, value) {
                    ("abicoder", Some("v1" | "v2")) => {}
                    ("experimental", Some("ABIEncoderV2")) => {}
                    ("experimental", Some("SMTChecker")) => {}
                    ("experimental", Some("solidity")) => {
                        self.dcx().err("experimental solidity features are not supported").emit();
                    }
                    _ => {
                        self.dcx().err("unknown pragma").span(span).emit();
                    }
                }
            }
            ast::PragmaTokens::Verbatim(_) => {
                self.dcx().err("unknown pragma").span(span).emit();
            }
        }
    }
}
