use crate::assert_js;

#[test]
fn zero_arity_custom_type() {
    assert_js!(
        r#"
type Mine{
    This
    That
}

fn go() {
    This
}
"#,
        r#""use strict";

function go() {
  return {type: "This"};
}
"#
    );
}

#[test]
fn custom_type_with_unnamed_fields() {
    assert_js!(
        r#"
type Ip{
    Ip(String)
}

fn build(x) {
    x("1.2.3.4")
}

fn go() {
    build(Ip)
    Ip("5.6.7.8")
}

// I don't think this accessor syntax is valid
// fn access(ip: Ip) {
//     ip.0
// }
"#,
        r#""use strict";

function build(x) {
  return x("1.2.3.4");
}

function go() {
  build((var0) => { return {type: "Ip", 0: var0}; });
  return (var0) => { return {type: "Ip", 0: var0}; }("5.6.7.8");
}
"#
    );

    assert_js!(
        r#"
type TypeWithALongNameAndSeveralArguments{
    TypeWithALongNameAndSeveralArguments(String, String, String, String, String)
}


fn go() {
    TypeWithALongNameAndSeveralArguments
    // TypeWithALongNameAndSeveralArguments("foo", "bar", "XXXXXXXXXXXXXXXXXXX", "baz", "last")
}
"#,
        r#""use strict";

function go() {
  return (var0, var1, var2, var3, var4) => {
    return {
      type: "TypeWithALongNameAndSeveralArguments",
      0: var0,
      1: var1,
      2: var2,
      3: var3,
      4: var4
    };
  };
}
"#
    );
}
