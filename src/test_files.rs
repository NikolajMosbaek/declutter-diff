use crate::lang::Lang;

/// Whether a changed file belongs to the test layer: by path convention first, then,
/// for files outside test directories, by importing a test framework.
pub fn is_test_file(path: &str, content: &str) -> bool {
    is_test_path(path)
        || Lang::from_path(path).is_some_and(|lang| imports_test_framework(lang, content))
}

pub fn is_test_path(path: &str) -> bool {
    let mut components: Vec<&str> = path.split('/').collect();
    let Some(file) = components.pop() else {
        return false;
    };
    components.iter().any(|dir| is_test_dir(dir)) || is_test_file_name(file)
}

fn is_test_dir(dir: &str) -> bool {
    let lower = dir.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "test" | "tests" | "__tests__" | "spec" | "specs" | "__mocks__" | "__snapshots__"
    ) || dir.ends_with("Tests")
}

fn is_test_file_name(file: &str) -> bool {
    let stem = file.split('.').next().unwrap_or(file);
    // CamelCase suffixes: FooTests.swift, FooTest.kt, FooTests.cs. Not `FooSpec`: too
    // many real types end in Spec (`TypeSpec`); Quick and Kotest specs are found by import.
    let camel = ["Tests", "Test"]
        .iter()
        .any(|suffix| stem.len() > suffix.len() && stem.ends_with(suffix));
    // Dotted and snake_case markers: foo.test.ts, foo.spec.tsx, foo_test.go, test_foo.py.
    let marked = file.contains(".test.")
        || file.contains(".spec.")
        || stem.ends_with("_test")
        || stem.ends_with("_spec")
        // `test_foo.py` is pytest's convention; elsewhere `test_` is just a name.
        || (stem.starts_with("test_") && file.ends_with(".py"))
        || file == "conftest.py";
    camel || marked
}

fn imports_test_framework(lang: Lang, content: &str) -> bool {
    content.lines().map(str::trim_start).any(|line| match lang {
        Lang::Swift => {
            line.starts_with("@testable import ")
                || matches!(
                    line,
                    "import XCTest" | "import Testing" | "import Quick" | "import Nimble"
                )
        }
        Lang::Python => [
            "import pytest",
            "from pytest",
            "import unittest",
            "from unittest",
        ]
        .iter()
        .any(|import| line.starts_with(import)),
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            line.starts_with("import")
                && ["vitest", "@jest/globals", "node:test", "@testing-library/"]
                    .iter()
                    .any(|module| {
                        line.contains(&format!("'{module}"))
                            || line.contains(&format!("\"{module}"))
                    })
        }
        Lang::Kotlin => ["org.junit", "kotlin.test", "io.kotest", "io.mockk"]
            .iter()
            .any(|package| line.starts_with(&format!("import {package}"))),
        // Go keeps tests in `_test.go` files, and Rust inside source files, which the
        // test-block spans cover.
        Lang::Go | Lang::Rust => false,
    })
}
