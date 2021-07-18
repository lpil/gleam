use gleam_core::{
    erl,
    error::{Error, FileIoAction, FileKind, InvalidProjectNameReason},
    parse, Result,
};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::{env, io::Write};
use strum::{Display, EnumString, EnumVariantNames};

use crate::NewOptions;

const GLEAM_STDLIB_VERSION: &str = "0.16.0";
const GLEAM_OTP_VERSION: &str = "0.1.5";
const ERLANG_OTP_VERSION: &str = "23.2";
const PROJECT_VERSION: &str = "0.1.0";

#[derive(Debug, Serialize, Deserialize, Display, EnumString, EnumVariantNames, Clone, Copy)]
#[strum(serialize_all = "kebab_case")]
pub enum Template {
    Lib,
    App,
    GleamLib,
    Escript,
}

#[derive(Debug, Serialize, Deserialize, Display, EnumString, EnumVariantNames, Clone, Copy)]
#[strum(serialize_all = "kebab_case")]
pub enum Docker {
    Default,
    Slim,
    Alpine,
}

#[derive(Debug)]
pub struct Creator {
    root: PathBuf,
    src: PathBuf,
    test: PathBuf,
    github: PathBuf,
    workflows: PathBuf,
    gleam_version: &'static str,
    options: NewOptions,
    project_name: String,
}

impl Creator {
    fn new(options: NewOptions, project_name: String, gleam_version: &'static str) -> Self {
        let root = PathBuf::from(&options.project_root);
        let src = root.join("src");
        let test = root.join("test");
        let github = root.join(".github");
        let workflows = github.join("workflows");
        Self {
            root,
            src,
            test,
            github,
            workflows,
            gleam_version,
            options,
            project_name,
        }
    }

    fn run(&self) -> Result<()> {
        crate::fs::mkdir(&self.root)?;
        crate::fs::mkdir(&self.src)?;
        crate::fs::mkdir(&self.test)?;
        crate::fs::mkdir(&self.github)?;
        crate::fs::mkdir(&self.workflows)?;

        match self.options.template {
            Template::Lib => {
                self.gitignore()?;
                self.github_ci()?;
                self.lib_readme()?;
                self.gleam_toml()?;
                self.lib_rebar_config()?;
                self.erlang_app_src()?;
                self.src_module()?;
                self.test_module()?;
                self.docker_compose()?;
            }

            Template::App => {
                crate::fs::mkdir(&self.src.join(self.project_name.clone()))?;
                self.gitignore()?;
                self.github_ci()?;
                self.app_readme()?;
                self.gleam_toml()?;
                self.app_rebar_config()?;
                self.erlang_app_src()?;
                self.src_module()?;
                self.src_application_module()?;
                self.test_module()?;
                self.docker_compose()?;
            }

            Template::Escript => {
                self.gitignore()?;
                self.github_ci()?;
                self.escript_readme()?;
                self.gleam_toml()?;
                self.lib_rebar_config()?;
                self.erlang_app_src()?;
                self.src_escript_module()?;
                self.test_module()?;
                self.docker_compose()?;
            }

            Template::GleamLib => {
                self.gitignore()?;
                self.gleam_github_ci()?;
                self.gleam_lib_readme()?;
                self.gleam_gleam_toml()?;
                self.src_module()?;
                self.gleam_test_module()?;
                self.docker_compose()?;
            }
        }

        Ok(())
    }

    fn src_escript_module(&self) -> Result<()> {
        write(
            self.src.join(format!("{}.gleam", self.project_name)),
            &format!(
                r#"import gleam/list
import gleam/io

pub external type CharList

pub fn main(args: List(CharList)) {{
  let _args = list.map(args, char_list_to_string)
  io.println(hello_world())
}}

pub fn hello_world() -> String {{
  "Hello, from {}!"
}}

external fn char_list_to_string(CharList) -> String =
  "erlang" "list_to_binary"
"#,
                self.project_name
            ),
        )
    }

    fn src_application_module(&self) -> Result<()> {
        write(
            self.src.join(&self.project_name).join("application.gleam"),
            r#"import gleam/otp/supervisor.{ApplicationStartMode, ErlangStartResult}
import gleam/dynamic.{Dynamic}

fn init(children) {
  children
}

pub fn start(
  _mode: ApplicationStartMode,
  _args: List(Dynamic),
) -> ErlangStartResult {
  init
  |> supervisor.start
  |> supervisor.to_erlang_start_result
}

pub fn stop(_state: Dynamic) {
  supervisor.application_stopped()
}
"#,
        )
    }

    fn src_module(&self) -> Result<()> {
        write(
            self.src.join(format!("{}.gleam", self.project_name)),
            &format!(
                r#"pub fn hello_world() -> String {{
  "Hello, from {}!"
}}
"#,
                self.project_name
            ),
        )
    }

    fn lib_rebar_config(&self) -> Result<()> {
        write(
            self.root.join("rebar.config"),
            &format!(
                r#"{{erl_opts, [debug_info]}}.
{{src_dirs, ["src", "gen/src"]}}.

{{profiles, [
    {{test, [{{src_dirs, ["src", "test", "gen/src", "gen/test"]}}]}}
]}}.

{{project_plugins, [rebar_gleam]}}.

{{deps, [
    {{gleam_stdlib, "{stdlib}"}}
]}}.
"#,
                stdlib = GLEAM_STDLIB_VERSION,
            ),
        )
    }

    fn app_rebar_config(&self) -> Result<()> {
        write(
            self.root.join("rebar.config"),
            &format!(
                r#"{{erl_opts, [debug_info]}}.
{{src_dirs, ["src", "gen/src"]}}.

{{profiles, [
    {{test, [{{src_dirs, ["src", "test", "gen/src", "gen/test"]}}]}}
]}}.

{{shell, [
    % {{config, "config/sys.config"}},
    {{apps, [{name}]}}
]}}.

{{project_plugins, [rebar_gleam]}}.

{{deps, [
    {{gleam_stdlib, "{stdlib}"}},
    {{gleam_otp, "{otp}"}}
]}}.
"#,
                name = self.project_name,
                stdlib = GLEAM_STDLIB_VERSION,
                otp = GLEAM_OTP_VERSION,
            ),
        )
    }

    fn erlang_app_src(&self) -> Result<()> {
        let module = match self.options.template {
            Template::App => format!("\n  {{mod, {{{}@application, []}}}},", self.project_name),
            _ => "".to_string(),
        };

        write(
            self.src.join(format!("{}.app.src", self.project_name)),
            &format!(
                r#"{{application, {application},
 [{{description, "{description}"}},
  {{vsn, "{version}"}},
  {{registered, []}},{module}
  {{applications,
   [kernel,
    stdlib,
    gleam_stdlib
   ]}},
  {{env,[]}},
  {{modules, []}},

  {{include_files, ["gleam.toml", "gen"]}},
  {{links, []}}
]}}.
"#,
                application = self.project_name,
                description = &self.options.description,
                version = PROJECT_VERSION,
                module = module,
            ),
        )
    }

    fn gitignore(&self) -> Result<()> {
        write(
            self.root.join(".gitignore"),
            "*.beam
*.iml
*.o
*.plt
*.swo
*.swp
*~
.erlang.cookie
.eunit
.idea
.rebar
.rebar3
_*
_build
docs
ebin
erl_crash.dump
gen
log
logs
rebar3.crashdump
",
        )
    }

    fn app_readme(&self) -> Result<()> {
        write(
            self.root.join("README.md"),
            &format!(
                r#"# {name}

{description}

## Quick start
{docker_run}
```sh
# Run the eunit tests
rebar3 eunit

# Run the Erlang REPL
rebar3 shell
```
"#,
                name = self.project_name,
                description = self.options.description,
                docker_run = self.docker_run()
            ),
        )
    }

    fn escript_readme(&self) -> Result<()> {
        write(
            self.root.join("README.md"),
            &format!(
                r#"# {name}

{description}

## Quick start
{docker_run}
```sh
# Run the eunit tests
rebar3 eunit

# Run the Erlang REPL
rebar3 shell

# Build and run the escript
rebar3 escriptize
_build/default/bin/{name}
```
"#,
                name = self.project_name,
                description = self.options.description,
                docker_run = self.docker_run()
            ),
        )
    }

    fn gleam_lib_readme(&self) -> Result<()> {
        write(
            self.root.join("README.md"),
            &format!(
                r#"# {name}

{description}

## Quick start
{docker_run}
```sh
# Run the eunit tests
gleam eunit

# Run the Erlang REPL
gleam shell
```
```
"#,
                name = self.project_name,
                description = self.options.description,
                docker_run = self.docker_run()
            ),
        )
    }

    fn lib_readme(&self) -> Result<()> {
        write(
            self.root.join("README.md"),
            &format!(
                r#"# {name}

{description}

## Quick start
{docker_run}
```sh
# Run the eunit tests
rebar3 eunit

# Run the Erlang REPL
rebar3 shell
```

## Installation

If [available in Hex](https://rebar3.org/docs/configuration/dependencies/#declaring-dependencies)
this package can be installed by adding `{name}` to your `rebar.config` dependencies:

```erlang
{{deps, [
    {name}
]}}.
```
"#,
                name = self.project_name,
                description = self.options.description,
                docker_run = self.docker_run()
            ),
        )
    }

    fn gleam_github_ci(&self) -> Result<()> {
        write(
            self.workflows.join("test.yml"),
            &format!(
                r#"name: test

on:
  push:
    branches:
      - master
      - main
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2.0.0
      - uses: gleam-lang/setup-erlang@v1.1.2
        with:
          otp-version: {}
      - uses: gleam-lang/setup-gleam@v1.0.2
        with:
          gleam-version: {}
      - run: gleam eunit
      - run: gleam format --check src test
"#,
                ERLANG_OTP_VERSION, self.gleam_version
            ),
        )
    }

    fn github_ci(&self) -> Result<()> {
        write(
            self.workflows.join("test.yml"),
            &format!(
                r#"name: test

on:
  push:
    branches:
      - master
      - main
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2.0.0
      - uses: gleam-lang/setup-erlang@v1.1.2
        with:
          otp-version: {}
      - uses: gleam-lang/setup-gleam@v1.0.2
        with:
          gleam-version: {}
      - run: rebar3 install_deps
      - run: rebar3 eunit
      - run: gleam format --check src test
"#,
                ERLANG_OTP_VERSION, self.gleam_version
            ),
        )
    }

    fn gleam_gleam_toml(&self) -> Result<()> {
        write(
            self.root.join("gleam.toml"),
            &format!(
                r#"name = "{name}"
tool = "gleam"
version = "0.1.0"
description = "A Gleam library"
"#,
                name = self.project_name,
            ),
        )
    }

    fn gleam_toml(&self) -> Result<()> {
        write(
            self.root.join("gleam.toml"),
            &format!(
                r#"name = "{name}"

# repository = {{ type = "github", user = "my-user", repo = "{name}" }}
"#,
                name = self.project_name,
            ),
        )
    }

    fn gleam_test_module(&self) -> Result<()> {
        write(
            self.test.join(format!("{}_test.gleam", self.project_name)),
            &format!(
                r#"import {name}

pub fn hello_world_test() {{
  assert "Hello, from {name}!" = {name}.hello_world()
  Nil
}}
"#,
                name = self.project_name
            ),
        )
    }

    fn test_module(&self) -> Result<()> {
        write(
            self.test.join(format!("{}_test.gleam", self.project_name)),
            &format!(
                r#"import {name}
import gleam/should

pub fn hello_world_test() {{
  {name}.hello_world()
  |> should.equal("Hello, from {name}!")
}}
"#,
                name = self.project_name
            ),
        )
    }

    fn docker_compose(&self) -> Result<()> {
        match self.options.docker {
            Some(choice) => {
                let docker_type = match choice.unwrap_or(Docker::Default) {
                    Docker::Default => "",
                    Docker::Slim => "-slim",
                    Docker::Alpine => "-alpine",
                };
                write(
                    self.root.join("docker-compose.yml"),
                    &format!(
                        r#"version: "3"
    
services:
  {name}:
    image: ghcr.io/gleam-lang/gleam:v{version}-erlang{docker_type}
    volumes:
      - .:/opt/app
    working_dir: /opt/app
    network_mode: host
    "#,
                        name = self.project_name,
                        version = self.gleam_version,
                        docker_type = docker_type
                    ),
                )
            }
            _ => Ok(()),
        }
    }

    fn docker_run(&self) -> String {
        format!(
            "\n```sh\ndocker-compose run {} bash\n```\n",
            self.project_name
        )
    }
}

pub fn create(options: NewOptions, version: &'static str) -> Result<()> {
    let name = if let Some(name) = options.name.clone() {
        name
    } else {
        get_foldername(&options.project_root)?
    };
    validate_name(name.as_str())?;
    validate_root_folder(name.as_str())?;
    let creator = Creator::new(options.clone(), name, version);

    creator.run()?;

    let cd_folder = if options.project_root == "." {
        "".to_string()
    } else {
        format!("\tcd {}\n", creator.options.project_root)
    };

    let docker_run = if options.docker.is_none() {
        "".to_string()
    } else {
        format!("\tdocker-compose run {} bash\n", creator.project_name)
    };

    let test_command = match &creator.options.template {
        Template::Lib | Template::App | Template::Escript => "rebar3 eunit",
        Template::GleamLib => "gleam eunit",
    };

    println!(
        "
Your Gleam project {} has been successfully created.
The project can be compiled and tested by running these commands:

{}{}\t{}
",
        creator.project_name, cd_folder, docker_run, test_command,
    );
    Ok(())
}

fn write(path: PathBuf, contents: &str) -> Result<()> {
    let mut f = File::create(&path).map_err(|err| Error::FileIo {
        kind: FileKind::File,
        path: path.clone(),
        action: FileIoAction::Create,
        err: Some(err.to_string()),
    })?;

    f.write_all(contents.as_bytes())
        .map_err(|err| Error::FileIo {
            kind: FileKind::File,
            path,
            action: FileIoAction::WriteTo,
            err: Some(err.to_string()),
        })?;
    Ok(())
}

fn validate_root_folder(name: &str) -> Result<(), Error> {
    if Path::new(name).exists() {
        Err(Error::ProjectRootAlreadyExist {
            path: name.to_string(),
        })
    } else {
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), Error> {
    if erl::is_erlang_reserved_word(name) {
        Err(Error::InvalidProjectName {
            name: name.to_string(),
            reason: InvalidProjectNameReason::ErlangReservedWord,
        })
    } else if erl::is_erlang_standard_library_module(name) {
        Err(Error::InvalidProjectName {
            name: name.to_string(),
            reason: InvalidProjectNameReason::ErlangStandardLibraryModule,
        })
    } else if parse::lexer::str_to_keyword(name).is_some() {
        Err(Error::InvalidProjectName {
            name: name.to_string(),
            reason: InvalidProjectNameReason::GleamReservedWord,
        })
    } else if name == "gleam" {
        Err(Error::InvalidProjectName {
            name: name.to_string(),
            reason: InvalidProjectNameReason::GleamReservedModule,
        })
    } else if !regex::Regex::new("^[a-z][a-z0-9_]*$")
        .expect("new name regex could not be compiled")
        .is_match(name)
    {
        Err(Error::InvalidProjectName {
            name: name.to_string(),
            reason: InvalidProjectNameReason::Format,
        })
    } else {
        Ok(())
    }
}

fn get_foldername(path: &str) -> Result<String, Error> {
    match path {
        "." => env::current_dir()
            .expect("invalid folder")
            .file_name()
            .and_then(|x| x.to_str())
            .map(ToString::to_string)
            .ok_or(Error::UnableToFindProjectRoot {
                path: path.to_string(),
            }),
        _ => Path::new(path)
            .file_name()
            .and_then(|x| x.to_str())
            .map(ToString::to_string)
            .ok_or(Error::UnableToFindProjectRoot {
                path: path.to_string(),
            }),
    }
}
