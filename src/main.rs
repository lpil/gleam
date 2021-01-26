#![deny(warnings)]
#![warn(
    clippy::all,
    clippy::pedantic,
    clippy::dbg_macro,
    clippy::todo,
    clippy::enum_glob_use,
    // TODO: enable once the false positive bug is solved
    // clippy::use_self,
    clippy::needless_continue,
    clippy::needless_borrow,
    clippy::if_let_mutex,
    clippy::match_on_vec_items,
    clippy::lossy_float_literal,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::inefficient_to_string,
    clippy::verbose_file_reads,
    rust_2018_idioms,
    missing_debug_implementations,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    nonstandard_style,
    unused_import_braces,
    unused_qualifications,
    unused_results,
    // Safety
    unsafe_code,
    clippy::unimplemented,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::integer_division,
    clippy::indexing_slicing,
    clippy::mem_forget
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::non_ascii_literal,
    clippy::too_many_lines,
    clippy::explicit_into_iter_loop
)]

#[macro_use]
mod pretty;

mod ast;
mod bit_string;
mod build;
mod cli;
mod codegen;
mod config;
mod diagnostic;
mod docs;
mod erl;
mod error;
mod eunit;
mod format;
mod fs;
mod metadata;
mod new;
mod num_util;
mod parse;
mod project;
mod shell;
mod typ;
mod warning;

mod schema_capnp {
    #![allow(
        dead_code,
        unused_qualifications,
        clippy::unseparated_literal_suffix,
        clippy::used_underscore_binding
    )]
    include!("../generated/schema_capnp.rs");
}

#[macro_use]
extern crate im;

#[cfg(test)]
#[macro_use]
extern crate pretty_assertions;

#[macro_use]
extern crate lazy_static;

pub use self::{
    error::{Error, GleamExpect, Result},
    warning::Warning,
};

use self::build::package_compiler;

use std::path::PathBuf;
use structopt::{clap::AppSettings, StructOpt};
use strum::VariantNames;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(StructOpt, Debug)]
#[structopt(global_settings = &[AppSettings::ColoredHelp, AppSettings::VersionlessSubcommands])]
enum Command {
    #[structopt(
        name = "build",
        about = "Compile a project",
        setting = AppSettings::Hidden,
    )]
    Build {
        #[structopt(help = "location of the project root", default_value = ".")]
        project_root: String,
    },

    #[structopt(name = "docs", about = "Render HTML documentation")]
    Docs(Docs),

    #[structopt(name = "new", about = "Create a new project")]
    New(NewOptions),

    #[structopt(name = "format", about = "Format source code")]
    Format {
        #[structopt(help = "files to format", default_value = ".")]
        files: Vec<String>,

        #[structopt(help = "read source from standard in", long = "stdin")]
        stdin: bool,

        #[structopt(
            help = "check if inputs are formatted without changing them",
            long = "check"
        )]
        check: bool,
    },

    #[structopt(
        name = "shell",
        about = "Start an Erlang shell",
        setting = AppSettings::Hidden,
    )]
    Shell {
        #[structopt(help = "location of the project root", default_value = ".")]
        project_root: String,
    },

    #[structopt(
        name = "eunit",
        about = "Run eunit tests",
        setting = AppSettings::Hidden,
    )]
    Eunit {
        #[structopt(help = "location of the project root", default_value = ".")]
        project_root: String,
    },

    #[structopt(
        name = "compile-package",
        about = "Compile a single Gleam package",
        setting = AppSettings::Hidden,
    )]
    CompilePackage(CompilePackage),
}

#[derive(StructOpt, Debug)]
#[structopt(flatten)]
pub struct NewOptions {
    #[structopt(help = "name of the project")]
    pub name: String,

    #[structopt(
        long = "description",
        help = "description of the project",
        default_value = "A Gleam project"
    )]
    pub description: String,

    #[structopt(help = "location of the project root")]
    pub project_root: Option<String>,

    #[structopt(
            long = "template",
            possible_values = &new::Template::VARIANTS,
            case_insensitive = true,
            default_value = "lib"
        )]
    pub template: new::Template,
}

#[derive(StructOpt, Debug)]
#[structopt(flatten)]
pub struct CompilePackage {
    #[structopt(help = "The name of the package being compiled", long = "name")]
    package_name: String,

    #[structopt(help = "A directory of source Gleam code", long = "src")]
    src_directory: PathBuf,

    #[structopt(help = "A directory of test Gleam code", long = "test")]
    test_directory: Option<PathBuf>,

    #[structopt(help = "A directory to write compiled code to", long = "out")]
    output_directory: PathBuf,

    #[structopt(help = "A path to a compiled dependency library", long = "lib")]
    libraries: Vec<PathBuf>,
}

impl CompilePackage {
    #[must_use]
    pub fn into_package_compiler_options(self) -> package_compiler::Options {
        package_compiler::Options {
            name: self.package_name,
            src_path: self.src_directory,
            test_path: self.test_directory,
            out_path: self.output_directory,
        }
    }
}

#[derive(StructOpt, Debug)]
enum Docs {
    #[structopt(name = "build", about = "Render HTML docs locally")]
    Build {
        #[structopt(help = "location of the project root", default_value = ".")]
        project_root: String,

        #[structopt(help = "the directory to write the docs to", long = "to")]
        to: Option<String>,

        #[structopt(help = "the version to publish", long = "version")]
        version: String,
    },

    #[structopt(name = "publish", about = "Publish HTML docs to HexDocs")]
    Publish {
        #[structopt(help = "location of the project root", default_value = ".")]
        project_root: String,

        #[structopt(help = "the version to publish", long = "version")]
        version: String,
    },

    #[structopt(name = "remove", about = "Remove HTML docs from HexDocs")]
    Remove {
        #[structopt(help = "the name of the package", long = "package")]
        package: String,

        #[structopt(help = "the version of the docs to remove", long = "version")]
        version: String,
    },
}

fn main() {
    initialise_logger();

    let result = match Command::from_args() {
        Command::Build { project_root } => command_build(&project_root),

        Command::Docs(Docs::Build {
            project_root,
            version,
            to,
        }) => docs::command::build(&project_root, &version, to),

        Command::Docs(Docs::Publish {
            project_root,
            version,
        }) => docs::command::publish(project_root, &version),

        Command::Docs(Docs::Remove { package, version }) => {
            docs::command::remove(&package, &version)
        }

        Command::Format {
            stdin,
            files,
            check,
        } => format::command::run(stdin, check, files),

        Command::New(options) => new::create(options, VERSION),

        Command::Shell { project_root } => shell::command(project_root),

        Command::Eunit { project_root } => eunit::command(project_root),

        Command::CompilePackage(opts) => build::compile_package::command(opts),
    };

    match result {
        Ok(_) => {
            tracing::info!("Successfully completed");
        }
        Err(error) => {
            tracing::error!(error = ?error, "Failed");
            error.pretty_print();
            std::process::exit(1);
        }
    }
}

fn command_build(root: &str) -> Result<(), Error> {
    let root = PathBuf::from(&root);
    let config = config::read_project_config(&root)?;

    // Use new build tool
    if config.tool == config::BuildTool::Gleam {
        return build::main(config, root).map(|_| ());
    }

    // Read and type check project
    let (_, analysed) = project::read_and_analyse(&root)?;

    // Generate Erlang code
    let output_files = erl::generate_erlang(analysed.as_slice());

    // Reset output directory
    fs::delete_dir(&root.join(project::OUTPUT_DIR_NAME))?;

    // Print warnings
    warning::print_all(analysed.as_slice());

    // Delete the gen directory before generating the newly compiled files
    fs::write_outputs(output_files.as_slice())?;

    println!("Done!");

    Ok(())
}

fn initialise_logger() {
    tracing_subscriber::fmt()
        .with_env_filter(&std::env::var("GLEAM_LOG").unwrap_or_else(|_| "off".to_string()))
        .with_target(false)
        .without_time()
        .init();
}
