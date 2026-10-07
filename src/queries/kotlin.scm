; Syntax colours for Kotlin. tree-sitter-kotlin-ng ships no highlight query, so this is a
; small one covering what the viewer colours: comments, strings, numbers and keywords.
; Every name here must exist in the grammar, or the whole query fails to compile.

(line_comment) @comment
(block_comment) @comment

(string_literal) @string
(multiline_string_literal) @string
(character_literal) @string

(number_literal) @number
(float_literal) @number

[
  "abstract" "actual" "annotation" "as" "by" "catch" "class" "companion" "const"
  "constructor" "crossinline" "data" "do" "else" "enum" "expect" "external" "final"
  "finally" "for" "fun" "if" "import" "in" "infix" "init" "inline" "inner" "interface"
  "internal" "is" "lateinit" "noinline" "object" "open" "operator" "out" "override"
  "package" "private" "protected" "public" "return" "sealed" "super" "suspend" "tailrec"
  "this" "throw" "try" "typealias" "val" "var" "vararg" "when" "where" "while"
] @keyword
