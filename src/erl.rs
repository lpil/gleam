#[cfg(test)]
mod tests;

use crate::{
    ast::*,
    error::GleamExpect,
    fs::{OutputFile, Utf8Writer},
    pretty::*,
    project::{self, Analysed},
    typ::{
        ModuleValueConstructor, PatternConstructor, Type, ValueConstructor, ValueConstructorVariant,
    },
    Result,
};
use heck::SnakeCase;
use itertools::Itertools;
use std::char;
use std::default::Default;
use std::sync::Arc;

const INDENT: isize = 4;

pub fn generate_erlang(analysed: &[Analysed]) -> Vec<OutputFile> {
    let mut files = Vec::with_capacity(analysed.len() * 2);

    for Analysed {
        name,
        origin,
        source_base_path,
        ast,
        ..
    } in analysed
    {
        let gen_dir = source_base_path
            .parent()
            .unwrap()
            .join(project::OUTPUT_DIR_NAME)
            .join(origin.dir_name());
        let erl_module_name = name.join("@");

        for (name, text) in records(ast).into_iter() {
            files.push(OutputFile {
                path: gen_dir.join(format!("{}_{}.hrl", erl_module_name, name)),
                text,
            })
        }

        let mut text = String::new();
        module(ast, &mut text).gleam_expect("Buffer writing failed");
        files.push(OutputFile {
            path: gen_dir.join(format!("{}.erl", erl_module_name)),
            text,
        });
    }

    files
}

#[derive(Debug, Clone)]
struct Env<'a> {
    module: &'a [String],
    current_scope_vars: im::HashMap<String, usize>,
    erl_function_scope_vars: im::HashMap<String, usize>,
}

impl<'env> Env<'env> {
    pub fn new(module: &'env [String]) -> Self {
        Self {
            current_scope_vars: Default::default(),
            erl_function_scope_vars: Default::default(),
            module,
        }
    }

    pub fn local_var_name<'a>(&mut self, name: String) -> Document<'a> {
        match self.current_scope_vars.get(&name) {
            None => {
                self.current_scope_vars.insert(name.clone(), 0);
                self.erl_function_scope_vars.insert(name.clone(), 0);
                variable_name(name).to_doc()
            }
            Some(0) => variable_name(name).to_doc(),
            Some(n) => variable_name(name).to_doc().append("@").append(*n),
        }
    }

    pub fn next_local_var_name<'a>(&mut self, name: String) -> Document<'a> {
        let next = self.erl_function_scope_vars.get(&name).map_or(0, |i| i + 1);
        self.erl_function_scope_vars.insert(name.clone(), next);
        self.current_scope_vars.insert(name.clone(), next);
        self.local_var_name(name)
    }
}

pub fn records(module: &TypedModule) -> Vec<(&str, String)> {
    module
        .statements
        .iter()
        .flat_map(|s| match s {
            Statement::CustomType {
                public: true,
                constructors,
                ..
            } => &constructors[..],
            _ => &[],
        })
        .filter(|constructor| !constructor.args.is_empty())
        .flat_map(|constructor| {
            let mut fields = Vec::with_capacity(constructor.args.len());
            for (label, ..) in constructor.args.iter() {
                match label {
                    Some(s) => fields.push(&**s),
                    None => return None,
                }
            }
            Some((&constructor.name, fields))
        })
        .map(|(name, fields)| (name.as_ref(), record_definition(name, &fields[..])))
        .collect()
}

pub fn record_definition(name: &str, fields: &[&str]) -> String {
    let name = &name.to_snake_case();
    let escaped_name = if is_erlang_reserved_word(name) {
        format!("'{}'", name)
    } else {
        name.to_string()
    };
    use std::fmt::Write;
    let mut buffer = format!("-record({}, {{", escaped_name);
    for field in fields.iter().intersperse(&", ") {
        let escaped_field = if is_erlang_reserved_word(field) {
            format!("'{}'", field)
        } else {
            (*field).to_string()
        };
        write!(buffer, "{}", escaped_field).unwrap();
    }
    writeln!(buffer, "}}).").unwrap();
    buffer
}

pub fn module(module: &TypedModule, writer: &mut impl Utf8Writer) -> Result<()> {
    let module_name = module.name.as_slice();
    let exports = concat(
        module
            .statements
            .iter()
            .flat_map(|s| match s {
                Statement::Fn {
                    public: true,
                    name,
                    args,
                    ..
                } => Some((name.clone(), args.len())),

                Statement::ExternalFn {
                    public: true,
                    name,
                    args,
                    ..
                } => Some((name.clone(), args.len())),

                _ => None,
            })
            .map(|(n, a)| atom(n).append("/").append(a))
            .intersperse(", ".to_doc()),
    );

    let statements = concat(
        module
            .statements
            .iter()
            .flat_map(|s| statement(s, module_name))
            .intersperse(lines(2)),
    );

    format!("-module({}).", module_name.join("@"))
        .to_doc()
        .append(line())
        .append("-compile(no_auto_import).")
        .append(lines(2))
        .append(if exports.is_nil() {
            nil()
        } else {
            "-export(["
                .to_doc()
                .append(exports)
                .append("]).")
                .append(lines(2))
        })
        .append(statements)
        .append(line())
        .pretty_print(80, writer)
}

fn statement<'a>(statement: &'a TypedStatement, module: &'a [String]) -> Option<Document<'a>> {
    match statement {
        Statement::TypeAlias { .. } => None,
        Statement::CustomType { .. } => None,
        Statement::Import { .. } => None,
        Statement::ExternalType { .. } => None,
        Statement::ModuleConstant { .. } => None,

        Statement::Fn {
            args, name, body, ..
        } => Some(mod_fun(name.as_ref(), args.as_slice(), body, module)),

        Statement::ExternalFn { public: false, .. } => None,
        Statement::ExternalFn {
            fun,
            module,
            args,
            name,
            ..
        } => Some(external_fun(
            name.as_ref(),
            module.as_ref(),
            fun.as_ref(),
            args.len(),
        )),
    }
}

fn mod_fun<'a>(
    name: &'a str,
    args: &'a [TypedArg],
    body: &'a TypedExpr,
    module: &'a [String],
) -> Document<'a> {
    let mut env = Env::new(module);

    atom(name.to_string())
        .append(fun_args(args, &mut env))
        .append(" ->")
        .append(line().append(expr(body, &mut env)).nest(INDENT).group())
        .append(".")
}

fn fun_args<'a>(args: &'a [TypedArg], env: &mut Env<'_>) -> Document<'a> {
    wrap_args(args.iter().map(|a| match &a.names {
        ArgNames::Discard { .. } | ArgNames::LabelledDiscard { .. } => "_".to_doc(),
        ArgNames::Named { name } | ArgNames::NamedLabelled { name, .. } => {
            env.next_local_var_name(name.to_string())
        }
    }))
}

fn wrap_args<'a, I>(args: I) -> Document<'a>
where
    I: Iterator<Item = Document<'a>>,
{
    break_("", "")
        .append(concat(args.intersperse(break_(",", ", "))))
        .nest(INDENT)
        .append(break_("", ""))
        .surround("(", ")")
        .group()
}

fn atom<'a>(value: String) -> Document<'a> {
    use regex::Regex;
    lazy_static! {
        static ref RE: Regex = Regex::new(r"^[a-z][a-z0-9_@]*$").unwrap();
    }

    match &*value {
        // Escape because of keyword collision
        value if is_erlang_reserved_word(value) => format!("'{}'", value).to_doc(),

        // No need to escape
        _ if RE.is_match(&value) => value.to_doc(),

        // Escape because of characters contained
        _ => value.to_doc().surround("'", "'"),
    }
}

fn string<'a>(value: &'a str) -> Document<'a> {
    value.to_doc().surround("<<\"", "\"/utf8>>")
}

fn tuple<'a>(elems: impl Iterator<Item = Document<'a>>) -> Document<'a> {
    concat(elems.intersperse(break_(",", ", ")))
        .nest_current()
        .surround("{", "}")
        .group()
}

fn bit_string<'a>(elems: impl Iterator<Item = Document<'a>>) -> Document<'a> {
    concat(elems.intersperse(break_(",", ", ")))
        .nest_current()
        .surround("<<", ">>")
        .group()
}

fn const_segment<'a>(
    value: &'a TypedConstant,
    options: &'a [BitStringSegmentOption<TypedConstant>],
    env: &mut Env<'_>,
) -> Document<'a> {
    let document = match value {
        // Skip the normal <<value/utf8>> surrounds
        Constant::String { value, .. } => value.clone().to_doc().surround("\"", "\""),

        // As normal
        Constant::Int { .. } | Constant::Float { .. } | Constant::BitString { .. } => {
            const_inline(value, env)
        }

        // Wrap anything else in parentheses
        value => const_inline(value, env).surround("(", ")"),
    };

    let size = |value: &'a TypedConstant, env: &mut Env<'_>| match value {
        Constant::Int { .. } => Some(":".to_doc().append(const_inline(value, env))),
        _ => Some(
            ":".to_doc()
                .append(const_inline(value, env).surround("(", ")")),
        ),
    };

    let unit = |value: &'a TypedConstant, env: &mut Env<'_>| match value {
        Constant::Int { .. } => Some("unit:".to_doc().append(const_inline(value, env))),
        _ => None,
    };

    bit_string_segment(document, options, size, unit, true, env)
}

fn expr_segment<'a>(
    value: &'a TypedExpr,
    options: &'a [BitStringSegmentOption<TypedExpr>],
    env: &mut Env<'_>,
) -> Document<'a> {
    let mut value_is_a_string_literal = false;

    let document = match value {
        // Skip the normal <<value/utf8>> surrounds and set the string literal flag
        TypedExpr::String { value, .. } => {
            value_is_a_string_literal = true;
            value.clone().to_doc().surround("\"", "\"")
        }

        // As normal
        TypedExpr::Int { .. }
        | TypedExpr::Float { .. }
        | TypedExpr::Var { .. }
        | TypedExpr::BitString { .. } => expr(value, env),

        // Wrap anything else in parentheses
        value => expr(value, env).surround("(", ")"),
    };

    let size = |value: &'a TypedExpr, env: &mut Env<'_>| match value {
        TypedExpr::Int { .. } | TypedExpr::Var { .. } => {
            Some(":".to_doc().append(expr(value, env)))
        }
        _ => Some(":".to_doc().append(expr(value, env).surround("(", ")"))),
    };

    let unit = |value: &'a TypedExpr, env: &mut Env<'_>| match value {
        TypedExpr::Int { .. } => Some("unit:".to_doc().append(expr(value, env))),
        _ => None,
    };

    bit_string_segment(
        document,
        options,
        size,
        unit,
        value_is_a_string_literal,
        env,
    )
}

fn pattern_segment<'a>(
    value: &'a TypedPattern,
    options: &'a [BitStringSegmentOption<TypedPattern>],
    env: &mut Env<'_>,
) -> Document<'a> {
    let document = match value {
        // Skip the normal <<value/utf8>> surrounds
        Pattern::String { value, .. } => value.clone().to_doc().surround("\"", "\""),

        // As normal
        Pattern::Discard { .. }
        | Pattern::Var { .. }
        | Pattern::Int { .. }
        | Pattern::Float { .. } => pattern(value, env),

        // No other pattern variants are allowed in pattern bit string segments
        _ => crate::error::fatal_compiler_bug("Pattern segment match not recognised"),
    };

    let size =
        |value: &'a TypedPattern, env: &mut Env<'_>| Some(":".to_doc().append(pattern(value, env)));

    let unit = |value: &'a TypedPattern, env: &mut Env<'_>| match value {
        Pattern::Int { .. } => Some("unit:".to_doc().append(pattern(value, env))),
        _ => None,
    };

    bit_string_segment(document, options, size, unit, true, env)
}

fn bit_string_segment<'a, Value: 'a, SizeToDoc, UnitToDoc>(
    mut document: Document<'a>,
    options: &'a [BitStringSegmentOption<Value>],
    mut size_to_doc: SizeToDoc,
    mut unit_to_doc: UnitToDoc,
    value_is_a_string_literal: bool,
    env: &mut Env<'_>,
) -> Document<'a>
where
    SizeToDoc: FnMut(&'a Value, &mut Env<'_>) -> Option<Document<'a>>,
    UnitToDoc: FnMut(&'a Value, &mut Env<'_>) -> Option<Document<'a>>,
{
    let mut size: Option<Document<'a>> = None;
    let mut unit: Option<Document<'a>> = None;
    let mut others = Vec::new();

    // Erlang only allows valid codepoint integers to be used as values for utf segments
    // We want to support <<string_var:utf8>> for all string variables, but <<StringVar/utf8>> is invalid
    // To work around this we use the binary type specifier for these segments instead
    let override_type = if !value_is_a_string_literal {
        Some("binary")
    } else {
        None
    };

    for option in options {
        match option {
            BitStringSegmentOption::Integer { .. } => others.push("integer"),
            BitStringSegmentOption::Float { .. } => others.push("float"),
            BitStringSegmentOption::Binary { .. } => others.push("binary"),
            BitStringSegmentOption::BitString { .. } => others.push("bitstring"),
            BitStringSegmentOption::UTF8 { .. } => others.push(override_type.unwrap_or("utf8")),
            BitStringSegmentOption::UTF16 { .. } => others.push(override_type.unwrap_or("utf16")),
            BitStringSegmentOption::UTF32 { .. } => others.push(override_type.unwrap_or("utf32")),
            BitStringSegmentOption::UTF8Codepoint { .. } => others.push("utf8"),
            BitStringSegmentOption::UTF16Codepoint { .. } => others.push("utf16"),
            BitStringSegmentOption::UTF32Codepoint { .. } => others.push("utf32"),
            BitStringSegmentOption::Signed { .. } => others.push("signed"),
            BitStringSegmentOption::Unsigned { .. } => others.push("unsigned"),
            BitStringSegmentOption::Big { .. } => others.push("big"),
            BitStringSegmentOption::Little { .. } => others.push("little"),
            BitStringSegmentOption::Native { .. } => others.push("native"),
            BitStringSegmentOption::Size { value, .. } => size = size_to_doc(value, env),
            BitStringSegmentOption::Unit { value, .. } => unit = unit_to_doc(value, env),
        }
    }

    document = document.append(size);

    if !others.is_empty() {
        document = document.append("/").append(others.join("-"));
    }

    if unit.is_some() {
        if !others.is_empty() {
            document = document.append("-").append(unit)
        } else {
            document = document.append("/").append(unit)
        }
    }

    document
}

fn seq<'a>(first: &'a TypedExpr, then: &'a TypedExpr, env: &mut Env<'_>) -> Document<'a> {
    force_break()
        .append(expr(first, env))
        .append(",")
        .append(line())
        .append(expr(then, env))
}

fn bin_op<'a>(
    name: &'a BinOp,
    left: &'a TypedExpr,
    right: &'a TypedExpr,
    env: &mut Env<'_>,
) -> Document<'a> {
    let div_zero = match name {
        BinOp::DivInt | BinOp::ModuloInt => Some("0"),
        BinOp::DivFloat => Some("0.0"),
        _ => None,
    };
    let op = match name {
        BinOp::And => "andalso",
        BinOp::Or => "orelse",
        BinOp::LtInt | BinOp::LtFloat => "<",
        BinOp::LtEqInt | BinOp::LtEqFloat => "=<",
        BinOp::Eq => "=:=",
        BinOp::NotEq => "/=",
        BinOp::GtInt | BinOp::GtFloat => ">",
        BinOp::GtEqInt | BinOp::GtEqFloat => ">=",
        BinOp::AddInt => "+",
        BinOp::AddFloat => "+",
        BinOp::SubInt => "-",
        BinOp::SubFloat => "-",
        BinOp::MultInt => "*",
        BinOp::MultFloat => "*",
        BinOp::DivInt => "div",
        BinOp::DivFloat => "/",
        BinOp::ModuloInt => "rem",
    };

    let left_expr = match left {
        TypedExpr::BinOp { .. } => expr(left, env).surround("(", ")"),
        _ => expr(left, env),
    };

    let right_expr = match right {
        TypedExpr::BinOp { .. } => expr(right, env).surround("(", ")"),
        _ => expr(right, env),
    };

    let div = |left: Document<'a>, right: Document<'a>| {
        left.append(break_("", " "))
            .append(op)
            .append(" ")
            .append(right)
    };

    match div_zero {
        Some(_) if right.non_zero_compile_time_number() => div(left_expr, right_expr),
        None => div(left_expr, right_expr),

        Some(zero) => {
            let denominator = "gleam@denominator";
            "case "
                .to_doc()
                .append(right_expr)
                .append(" of")
                .append(
                    line()
                        .append(zero)
                        .append(" -> ")
                        .append(zero)
                        .append(";")
                        .append(line())
                        .append(env.next_local_var_name(denominator.to_string()))
                        .append(" -> ")
                        .append(div(left_expr, env.local_var_name(denominator.to_string())))
                        .nest(INDENT),
                )
                .append(line())
                .append("end")
        }
    }
}

fn pipe<'a>(value: &'a TypedExpr, fun: &'a TypedExpr, env: &mut Env<'_>) -> Document<'a> {
    docs_args_call(fun, vec![expr(value, env)], env)
}

fn try_<'a>(
    value: &'a TypedExpr,
    pat: &'a TypedPattern,
    then: &'a TypedExpr,
    env: &mut Env<'_>,
) -> Document<'a> {
    let try_error_name = "gleam@try_error";

    "case "
        .to_doc()
        .append(expr(value, env))
        .append(" of")
        .append(
            line()
                .append("{error, ")
                .append(env.next_local_var_name(try_error_name.to_string()))
                .append("} -> {error, ")
                .append(env.local_var_name(try_error_name.to_string()))
                .append("};")
                .nest(INDENT),
        )
        .append(
            line()
                .append("{ok, ")
                .append(pattern(pat, env))
                .append("} ->")
                .append(line().append(expr(then, env)).nest(INDENT))
                .nest(INDENT),
        )
        .append(line())
        .append("end")
        .group()
}

fn let_<'a>(
    value: &'a TypedExpr,
    pat: &'a TypedPattern,
    then: &'a TypedExpr,
    env: &mut Env<'_>,
) -> Document<'a> {
    let body = maybe_block_expr(value, env);
    pattern(pat, env)
        .append(" = ")
        .append(body)
        .append(",")
        .append(line())
        .append(expr(then, env))
}

fn pattern<'a>(p: &'a TypedPattern, env: &mut Env<'_>) -> Document<'a> {
    match p {
        Pattern::Nil { .. } => "[]".to_doc(),

        Pattern::Let { name, pattern: p } => pattern(p, env)
            .append(" = ")
            .append(env.next_local_var_name(name.to_string())),

        Pattern::Cons { head, tail, .. } => pattern_list_cons(head, tail, env),

        Pattern::Discard { .. } => "_".to_doc(),

        Pattern::Var { name, .. } => env.next_local_var_name(name.to_string()),

        Pattern::VarCall { name, .. } => env.local_var_name(name.to_string()),

        Pattern::Int { value, .. } => int(value.as_ref()),

        Pattern::Float { value, .. } => float(value.as_ref()),

        Pattern::String { value, .. } => string(value),

        Pattern::Constructor {
            args,
            constructor: PatternConstructor::Record { name },
            ..
        } => tag_tuple_pattern(name, args, env),

        Pattern::Tuple { elems, .. } => tuple(elems.iter().map(|p| pattern(p, env))),

        Pattern::BitString { segments, .. } => bit_string(
            segments
                .iter()
                .map(|s| pattern_segment(&s.value, s.options.as_slice(), env)),
        ),
    }
}

fn float<'a>(value: &str) -> Document<'a> {
    let mut value = value.replace("_", "");
    if value.ends_with('.') {
        value.push('0')
    }
    value.to_doc()
}

fn pattern_list_cons<'a>(
    head: &'a TypedPattern,
    tail: &'a TypedPattern,
    env: &mut Env<'_>,
) -> Document<'a> {
    list_cons(head, tail, env, pattern, |expr| match expr {
        Pattern::Nil { .. } => ListType::Nil,

        Pattern::Cons { head, tail, .. } => ListType::Cons { head, tail },

        other => ListType::NotList(other),
    })
}

fn expr_list_cons<'a>(head: &'a TypedExpr, tail: &'a TypedExpr, env: &mut Env<'_>) -> Document<'a> {
    list_cons(head, tail, env, maybe_block_expr, |expr| match expr {
        TypedExpr::ListNil { .. } => ListType::Nil,

        TypedExpr::ListCons { head, tail, .. } => ListType::Cons { head, tail },

        other => ListType::NotList(other),
    })
}

fn list_cons<'a, ToDoc, Categorise, Elem: 'a>(
    head: Elem,
    tail: Elem,
    env: &mut Env<'_>,
    to_doc: ToDoc,
    categorise_element: Categorise,
) -> Document<'a>
where
    ToDoc: Fn(Elem, &mut Env<'_>) -> Document<'a>,
    Categorise: Fn(Elem) -> ListType<Elem, Elem>,
{
    let mut elems = vec![head];
    let final_tail = collect_cons(tail, &mut elems, categorise_element);

    let elems = concat(
        elems
            .into_iter()
            .map(|e| to_doc(e, env))
            .intersperse(break_(",", ", ")),
    );

    let elems = if let Some(final_tail) = final_tail {
        elems
            .append(break_(" |", " | "))
            .append(to_doc(final_tail, env))
    } else {
        elems
    };

    elems.to_doc().nest_current().surround("[", "]").group()
}

fn collect_cons<'a, F, E, T>(e: T, elems: &'a mut Vec<E>, f: F) -> Option<T>
where
    F: Fn(T) -> ListType<E, T>,
{
    match f(e) {
        ListType::Nil => None,

        ListType::Cons { head, tail } => {
            elems.push(head);
            collect_cons(tail, elems, f)
        }

        ListType::NotList(other) => Some(other),
    }
}

enum ListType<E, T> {
    Nil,
    Cons { head: E, tail: T },
    NotList(T),
}

fn var<'a>(name: &'a str, constructor: &'a ValueConstructor, env: &mut Env<'_>) -> Document<'a> {
    match &constructor.variant {
        ValueConstructorVariant::Record {
            name: record_name, ..
        } => match &*constructor.typ {
            Type::Fn { args, .. } => {
                let chars = incrementing_args_list(args.len());
                "fun("
                    .to_doc()
                    .append(chars.clone())
                    .append(") -> {")
                    .append(record_name.to_snake_case())
                    .append(", ")
                    .append(chars)
                    .append("} end")
            }
            _ => atom(record_name.to_snake_case()),
        },

        ValueConstructorVariant::LocalVariable => env.local_var_name(name.to_string()),

        ValueConstructorVariant::ModuleConstant { literal } => const_inline(literal, env),

        ValueConstructorVariant::ModuleFn {
            arity, ref module, ..
        } if module.as_slice() == env.module => "fun "
            .to_doc()
            .append(atom(name.to_string()))
            .append("/")
            .append(*arity),

        ValueConstructorVariant::ModuleFn {
            arity,
            module,
            name,
            ..
        } => "fun "
            .to_doc()
            .append(module.join("@"))
            .append(":")
            .append(atom(name.to_string()))
            .append("/")
            .append(*arity),
    }
}

fn int<'a>(value: &str) -> Document<'a> {
    value
        .replace("_", "")
        .replace("0x", "16#")
        .replace("0o", "8#")
        .replace("0b", "2#")
        .to_doc()
}

fn const_inline<'a>(literal: &'a TypedConstant, env: &mut Env<'_>) -> Document<'a> {
    match literal {
        Constant::Int { value, .. } => int(value),
        Constant::Float { value, .. } => float(value),
        Constant::String { value, .. } => string(value),
        Constant::Tuple { elements, .. } => tuple(elements.iter().map(|e| const_inline(e, env))),

        Constant::List { elements, .. } => {
            let elements = elements
                .iter()
                .map(|e| const_inline(e, env))
                .intersperse(break_(",", ", "));
            concat(elements).nest_current().surround("[", "]").group()
        }

        Constant::BitString { segments, .. } => bit_string(
            segments
                .iter()
                .map(|s| const_segment(&s.value, s.options.as_slice(), env)),
        ),

        Constant::Record { tag, args, .. } => {
            if args.is_empty() {
                atom(tag.to_snake_case())
            } else {
                let args = args.iter().map(|a| const_inline(&a.value, env));
                let tag = atom(tag.to_snake_case());
                tuple(std::iter::once(tag).chain(args))
            }
        }
    }
}

fn tag_tuple_pattern<'a>(
    name: &'a str,
    args: &'a [CallArg<TypedPattern>],
    env: &mut Env<'_>,
) -> Document<'a> {
    if args.is_empty() {
        atom(name.to_snake_case())
    } else {
        tuple(
            std::iter::once(atom(name.to_snake_case()))
                .chain(args.iter().map(|p| pattern(&p.value, env))),
        )
    }
}

fn clause<'a>(clause: &'a TypedClause, env: &mut Env<'_>) -> Document<'a> {
    let Clause {
        guard,
        pattern: pat,
        alternative_patterns,
        then,
        ..
    } = clause;

    // These are required to get the alternative patterns working properly.
    // Simply rendering the duplicate erlang clauses breaks the variable rewriting
    let mut then_doc = Document::Nil;
    let erlang_vars = env.erl_function_scope_vars.clone();

    let docs = std::iter::once(pat)
        .chain(alternative_patterns.iter())
        .map(|patterns| {
            env.erl_function_scope_vars = erlang_vars.clone();

            let patterns_doc = if patterns.len() == 1 {
                let p = patterns
                    .get(0)
                    .gleam_expect("Single pattern clause printing");
                pattern(p, env)
            } else {
                tuple(patterns.iter().map(|p| pattern(p, env)))
            };

            if then_doc == Document::Nil {
                then_doc = expr(then, env);
            }

            patterns_doc.append(
                optional_clause_guard(guard.as_ref(), env)
                    .append(" ->")
                    .append(line().append(then_doc.clone()).nest(INDENT).group()),
            )
        })
        .intersperse(";".to_doc().append(lines(2)));

    concat(docs)
}

fn optional_clause_guard<'a>(
    guard: Option<&'a TypedClauseGuard>,
    env: &mut Env<'_>,
) -> Document<'a> {
    match guard {
        Some(guard) => " when ".to_doc().append(bare_clause_guard(guard, env)),
        None => nil(),
    }
}

fn bare_clause_guard<'a>(guard: &'a TypedClauseGuard, env: &mut Env<'_>) -> Document<'a> {
    match guard {
        ClauseGuard::Or { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" orelse ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::And { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" andalso ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::Equals { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" =:= ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::NotEquals { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" =/= ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::GtInt { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" > ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::GtEqInt { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" >= ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::LtInt { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" < ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::LtEqInt { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" =< ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::GtFloat { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" > ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::GtEqFloat { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" >= ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::LtFloat { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" < ")
            .append(clause_guard(right.as_ref(), env)),

        ClauseGuard::LtEqFloat { left, right, .. } => clause_guard(left.as_ref(), env)
            .append(" =< ")
            .append(clause_guard(right.as_ref(), env)),

        // Only local variables are supported and the typer ensures that all
        // ClauseGuard::Vars are local variables
        ClauseGuard::Var { name, .. } => env.local_var_name(name.to_string()),

        ClauseGuard::TupleIndex { tuple, index, .. } => tuple_index_inline(tuple, *index, env),

        ClauseGuard::Constant(constant) => const_inline(constant, env),
    }
}

fn tuple_index_inline<'a>(
    tuple: &'a TypedClauseGuard,
    index: u64,
    env: &mut Env<'_>,
) -> Document<'a> {
    use std::iter::once;
    let index_doc = format!("{}", (index + 1)).to_doc();
    let tuple_doc = bare_clause_guard(tuple, env);
    let iter = once(index_doc).chain(once(tuple_doc));
    "erlang:element".to_doc().append(wrap_args(iter))
}

fn clause_guard<'a>(guard: &'a TypedClauseGuard, env: &mut Env<'_>) -> Document<'a> {
    match guard {
        // Binary ops are wrapped in parens
        ClauseGuard::Or { .. }
        | ClauseGuard::And { .. }
        | ClauseGuard::Equals { .. }
        | ClauseGuard::NotEquals { .. }
        | ClauseGuard::GtInt { .. }
        | ClauseGuard::GtEqInt { .. }
        | ClauseGuard::LtInt { .. }
        | ClauseGuard::LtEqInt { .. }
        | ClauseGuard::GtFloat { .. }
        | ClauseGuard::GtEqFloat { .. }
        | ClauseGuard::LtFloat { .. }
        | ClauseGuard::LtEqFloat { .. } => "("
            .to_doc()
            .append(bare_clause_guard(guard, env))
            .append(")"),

        // Values are not wrapped
        ClauseGuard::Constant(_) | ClauseGuard::Var { .. } | ClauseGuard::TupleIndex { .. } => {
            bare_clause_guard(guard, env)
        }
    }
}

fn clauses<'a>(cs: &'a [TypedClause], env: &mut Env<'_>) -> Document<'a> {
    concat(
        cs.iter()
            .map(|c| {
                let vars = env.current_scope_vars.clone();
                let erl = clause(c, env);
                env.current_scope_vars = vars; // Reset the known variables now the clauses' scope has ended
                erl
            })
            .intersperse(";".to_doc().append(lines(2))),
    )
}

fn case<'a>(subjects: &'a [TypedExpr], cs: &'a [TypedClause], env: &mut Env<'_>) -> Document<'a> {
    let subjects_doc = if subjects.len() == 1 {
        let subject = subjects
            .get(0)
            .gleam_expect("erl case printing of single subject");
        maybe_block_expr(subject, env).group()
    } else {
        tuple(subjects.iter().map(|e| maybe_block_expr(e, env)))
    };
    "case "
        .to_doc()
        .append(subjects_doc)
        .append(" of")
        .append(line().append(clauses(cs, env)).nest(INDENT))
        .append(line())
        .append("end")
        .group()
}

fn call<'a>(fun: &'a TypedExpr, args: &'a [CallArg<TypedExpr>], env: &mut Env<'_>) -> Document<'a> {
    docs_args_call(
        fun,
        args.iter()
            .map(|arg| maybe_block_expr(&arg.value, env))
            .collect(),
        env,
    )
}

fn docs_args_call<'a>(
    fun: &'a TypedExpr,
    mut args: Vec<Document<'a>>,
    env: &mut Env<'_>,
) -> Document<'a> {
    match fun {
        TypedExpr::ModuleSelect {
            constructor: ModuleValueConstructor::Record { name, .. },
            ..
        }
        | TypedExpr::Var {
            constructor:
                ValueConstructor {
                    variant: ValueConstructorVariant::Record { name, .. },
                    ..
                },
            ..
        } => tuple(std::iter::once(atom(name.to_snake_case())).chain(args.into_iter())),

        TypedExpr::Var {
            constructor:
                ValueConstructor {
                    variant: ValueConstructorVariant::ModuleFn { module, name, .. },
                    ..
                },
            ..
        } => {
            let args = wrap_args(args.into_iter());
            if module.as_slice() == env.module {
                atom(name.to_string()).append(args)
            } else {
                atom(module.join("@"))
                    .append(":")
                    .append(atom(name.to_string()))
                    .append(args)
            }
        }

        TypedExpr::ModuleSelect {
            module_name,
            label,
            constructor: ModuleValueConstructor::Fn,
            ..
        } => {
            let args = wrap_args(args.into_iter());
            atom(module_name.join("@"))
                .append(":")
                .append(atom(label.to_string()))
                .append(args)
        }

        TypedExpr::Fn {
            is_capture: true,
            body,
            ..
        } => {
            if let TypedExpr::Call {
                fun,
                args: inner_args,
                ..
            } = body.as_ref()
            {
                let mut merged_args = Vec::with_capacity(inner_args.len());
                for arg in inner_args.iter() {
                    match &arg.value {
                        TypedExpr::Var { name, .. } if name == CAPTURE_VARIABLE => {
                            merged_args.push(args.swap_remove(0))
                        }
                        e => merged_args.push(expr(e, env)),
                    }
                }
                docs_args_call(fun, merged_args, env)
            } else {
                crate::error::fatal_compiler_bug("Erl printing: Capture was not a call")
            }
        }

        TypedExpr::Call { .. }
        | TypedExpr::Fn { .. }
        | TypedExpr::RecordAccess { .. }
        | TypedExpr::TupleIndex { .. } => {
            let args = wrap_args(args.into_iter());
            expr(fun, env).surround("(", ")").append(args)
        }

        other => {
            let args = wrap_args(args.into_iter());
            expr(other, env).append(args)
        }
    }
}

fn record_update<'a>(
    spread: &'a TypedExpr,
    args: &'a [TypedRecordUpdateArg],
    env: &mut Env<'_>,
) -> Document<'a> {
    use std::iter::once;

    args.iter().fold(expr(spread, env), |tuple_doc, arg| {
        // Increment the index by 2, because the first element
        // is the name of the record, so our fields are 2-indexed
        let index_doc = (arg.index + 2).to_doc();
        let value_doc = expr(&arg.value, env);

        let iter = once(index_doc)
            .chain(once(tuple_doc))
            .chain(once(value_doc));

        "erlang:setelement".to_doc().append(wrap_args(iter))
    })
}

/// Wrap a document in begin end
///
fn begin_end<'a>(document: Document<'a>) -> Document<'a> {
    force_break()
        .append("begin")
        .append(line().append(document).nest(INDENT))
        .append(line())
        .append("end")
}

/// Same as expr, expect it wraps seq, let, etc in begin end
///
fn maybe_block_expr<'a>(expression: &'a TypedExpr, env: &mut Env<'_>) -> Document<'a> {
    match &expression {
        TypedExpr::Seq { .. } | TypedExpr::Let { .. } => begin_end(expr(expression, env)),
        _ => expr(expression, env),
    }
}

fn expr<'a>(expression: &'a TypedExpr, env: &mut Env<'_>) -> Document<'a> {
    match expression {
        TypedExpr::ListNil { .. } => "[]".to_doc(),

        TypedExpr::Todo { label: None, .. } => "erlang:error({gleam_error, todo})".to_doc(),

        TypedExpr::Todo { label: Some(l), .. } => l
            .clone()
            .to_doc()
            .surround("erlang:error({gleam_error, todo, \"", "\"})"),

        TypedExpr::Int { value, .. } => int(value.as_ref()),
        TypedExpr::Float { value, .. } => float(value.as_ref()),
        TypedExpr::String { value, .. } => string(value),
        TypedExpr::Seq { first, then, .. } => seq(first, then, env),
        TypedExpr::Pipe { left, right, .. } => pipe(left, right, env),

        TypedExpr::TupleIndex { tuple, index, .. } => tuple_index(tuple, *index, env),

        TypedExpr::Var {
            name, constructor, ..
        } => var(name, constructor, env),

        TypedExpr::Fn { args, body, .. } => fun(args, body, env),

        TypedExpr::ListCons { head, tail, .. } => expr_list_cons(head, tail, env),

        TypedExpr::Call { fun, args, .. } => call(fun, args, env),

        TypedExpr::ModuleSelect {
            constructor: ModuleValueConstructor::Record { name, arity: 0 },
            ..
        } => atom(name.to_snake_case()),

        TypedExpr::ModuleSelect {
            constructor: ModuleValueConstructor::Constant { literal },
            ..
        } => const_inline(literal, env),

        TypedExpr::ModuleSelect {
            constructor: ModuleValueConstructor::Record { name, arity },
            ..
        } => {
            let chars = incrementing_args_list(*arity);
            "fun("
                .to_doc()
                .append(chars.clone())
                .append(") -> {")
                .append(name.to_snake_case())
                .append(", ")
                .append(chars)
                .append("} end")
        }

        TypedExpr::ModuleSelect {
            typ,
            label,
            module_name,
            constructor: ModuleValueConstructor::Fn,
            ..
        } => module_select_fn(typ.clone(), module_name, label),

        TypedExpr::RecordAccess { record, index, .. } => tuple_index(record, index + 1, env),

        TypedExpr::RecordUpdate { spread, args, .. } => record_update(spread, args, env),

        TypedExpr::Let {
            value,
            pattern,
            then,
            kind: BindingKind::Try,
            ..
        } => try_(value, pattern, then, env),

        TypedExpr::Let {
            value,
            pattern,
            then,
            ..
        } => let_(value, pattern, then, env),

        TypedExpr::Case {
            subjects, clauses, ..
        } => case(subjects, clauses.as_slice(), env),

        TypedExpr::BinOp {
            name, left, right, ..
        } => bin_op(name, left, right, env),

        TypedExpr::Tuple { elems, .. } => tuple(elems.iter().map(|e| maybe_block_expr(e, env))),

        TypedExpr::BitString { segments, .. } => bit_string(
            segments
                .iter()
                .map(|s| expr_segment(&s.value, s.options.as_slice(), env)),
        ),
    }
}

fn tuple_index<'a>(tuple: &'a TypedExpr, index: u64, env: &mut Env<'_>) -> Document<'a> {
    use std::iter::once;
    let index_doc = format!("{}", (index + 1)).to_doc();
    let tuple_doc = expr(tuple, env);
    let iter = once(index_doc).chain(once(tuple_doc));
    "erlang:element".to_doc().append(wrap_args(iter))
}

fn module_select_fn<'a>(typ: Arc<Type>, module_name: &[String], label: &str) -> Document<'a> {
    match crate::typ::collapse_links(typ).as_ref() {
        crate::typ::Type::Fn { args, .. } => "fun "
            .to_doc()
            .append(module_name.join("@"))
            .append(":")
            .append(atom(label.to_string()))
            .append("/")
            .append(args.len()),

        _ => module_name
            .join("@")
            .to_doc()
            .append(":")
            .append(label.to_string())
            .append("()"),
    }
}

fn fun<'a>(args: &'a [TypedArg], body: &'a TypedExpr, env: &mut Env<'_>) -> Document<'a> {
    let current_scope_vars = env.current_scope_vars.clone();
    let doc = "fun"
        .to_doc()
        .append(fun_args(args, env).append(" ->"))
        .append(break_("", " ").append(expr(body, env)).nest(INDENT))
        .append(break_("", " "))
        .append("end")
        .group();
    env.current_scope_vars = current_scope_vars;
    doc
}

fn incrementing_args_list(arity: usize) -> String {
    (65..(65 + arity))
        .map(|x| x as u8 as char)
        .map(|c| c.to_string())
        .intersperse(", ".to_string())
        .collect()
}

fn external_fun<'a>(name: &str, module: &str, fun: &str, arity: usize) -> Document<'a> {
    let chars: String = incrementing_args_list(arity);

    atom(name.to_string())
        .append(format!("({}) ->", chars))
        .append(line())
        .append(atom(module.to_string()))
        .append(":")
        .append(atom(fun.to_string()))
        .append(format!("({}).", chars))
        .nest(INDENT)
}

fn variable_name(name: String) -> String {
    let mut c = name.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().chain(c).collect(),
    }
}

pub fn is_erlang_reserved_word(name: &str) -> bool {
    matches!(
        name,
        "!" | "receive"
            | "bnot"
            | "div"
            | "rem"
            | "band"
            | "bor"
            | "bxor"
            | "bsl"
            | "bsr"
            | "not"
            | "and"
            | "or"
            | "xor"
            | "orelse"
            | "andalso"
            | "when"
            | "end"
            | "fun"
            | "try"
            | "catch"
            | "after"
    )
}

// Includes shell_default & user_default which are looked for by the erlang shell
pub fn is_erlang_standard_library_module(name: &str) -> bool {
    matches!(
        name,
        "array"
            | "base64"
            | "beam_lib"
            | "binary"
            | "c"
            | "calendar"
            | "dets"
            | "dict"
            | "digraph"
            | "digraph_utils"
            | "epp"
            | "erl_anno"
            | "erl_eval"
            | "erl_expand_records"
            | "erl_id_trans"
            | "erl_internal"
            | "erl_lint"
            | "erl_parse"
            | "erl_pp"
            | "erl_scan"
            | "erl_tar"
            | "ets"
            | "file_sorter"
            | "filelib"
            | "filename"
            | "gb_sets"
            | "gb_trees"
            | "gen_event"
            | "gen_fsm"
            | "gen_server"
            | "gen_statem"
            | "io"
            | "io_lib"
            | "lists"
            | "log_mf_h"
            | "maps"
            | "math"
            | "ms_transform"
            | "orddict"
            | "ordsets"
            | "pool"
            | "proc_lib"
            | "proplists"
            | "qlc"
            | "queue"
            | "rand"
            | "random"
            | "re"
            | "sets"
            | "shell"
            | "shell_default"
            | "shell_docs"
            | "slave"
            | "sofs"
            | "string"
            | "supervisor"
            | "supervisor_bridge"
            | "sys"
            | "timer"
            | "unicode"
            | "uri_string"
            | "user_default"
            | "win32reg"
            | "zip"
    )
}
