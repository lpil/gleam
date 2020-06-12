import should

// Helpers
fn integer_fn() {
  1
}

// Valid values
pub fn function_as_value_test() {
  let <<a>> = <<integer_fn():int>>

  should.equal(a, 1)
}

pub fn integer_to_binary_test() {
  let <<a, rest:binary>> = <<1, 17, 42:16>>

  should.equal(a, 1)
  should.equal(rest, <<17, 0, 42>>)
}

// Sizes
pub fn size_variable_from_match_test() {
  let <<
    name_size:8,
    name:binary-size(name_size),
    " the ":utf8,
    species:binary,
  >> = <<5, "Frank the Walrus":utf8>>

  should.equal(name, <<"Frank":utf8>>)
  should.equal(species, <<"Walrus":utf8>>)
}

pub fn sizes_with_expressions() {
  let a = 1
  let b = <<a:unit(2)-size(a * 2), a:size(3 + integer_fn())-unit(1)>>

  should.equal(b, <<1, 1>>)
}

// Strings
pub fn string_test() {
  let a = "test"
  let <<b:2-binary, "st":utf8>> = a

  should.equal(b, <<"te":utf8>>)
}

pub fn explicit_utf8_test() {
  let a = <<"test":utf8>>
  let <<b:2-binary, "st":utf8>> = a

  should.equal(b, <<"te":utf8>>)
}

pub fn emoji_test() {
  let a = <<"😁😀":utf8>>
  let <<b:4-binary, "😀":utf8>> = a

  should.equal(b, <<"😁":utf8>>)
}
