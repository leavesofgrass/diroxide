//! syntect as a tokenizer: a line parsed into scope operations, the
//! scope stack walked, and each stretch between two operations given the
//! class of its stack (the scope mapper). syntect's highlighter, themes
//! and HTML output are not used; nothing here has a color.
//!
//! The syntax set (syntect's own grammars with bat's, from two-face) is
//! loaded on first use, never at startup, on whichever thread asks first:
//! the highlight worker's.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use syntect::parsing::{ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxSet};

use super::language::Language;
use super::scopes::{class_of, rule_of};
use crate::classes::{ClassRun, TokenClass};

static SET: OnceLock<SyntaxSet> = OnceLock::new();
static LOAD_TIME: OnceLock<Duration> = OnceLock::new();

/// The syntax set, loaded on first use.
pub(crate) fn syntax_set() -> &'static SyntaxSet {
    SET.get_or_init(|| {
        let started = Instant::now();
        let set = two_face::syntax::extra_newlines();
        let _ = LOAD_TIME.set(started.elapsed());
        set
    })
}

/// How long loading the syntax set took, once it has been loaded.
pub(crate) fn load_time() -> Option<Duration> {
    LOAD_TIME.get().copied()
}

/// What a named token is, from its scope: the symbols the outline lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NameKind {
    /// `entity.name.function`.
    Function,
    /// `entity.name.class`, `struct`, `enum`, `trait`, `type`, `union`,
    /// `interface`.
    Type,
    /// `entity.name.impl`.
    Impl,
    /// `entity.name.module`, `entity.name.namespace`.
    Module,
    /// `entity.name.section`: a heading.
    Section,
}

impl NameKind {
    fn of(scope: &str) -> Option<Self> {
        let rest = scope.strip_prefix("entity.name.")?;
        let first = rest.split('.').next().unwrap_or(rest);
        Some(match first {
            "function" => NameKind::Function,
            "class" | "struct" | "enum" | "trait" | "type" | "union" | "interface" => {
                NameKind::Type
            }
            "impl" => NameKind::Impl,
            "module" | "namespace" => NameKind::Module,
            "section" => NameKind::Section,
            _ => return None,
        })
    }
}

/// A token whose scope names something: its bytes and kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Named {
    pub start: u32,
    pub end: u32,
    pub kind: NameKind,
}

/// One line, tokenized.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineOut {
    pub runs: Vec<ClassRun>,
    pub names: Vec<Named>,
    /// Every scope seen in the line, by name, when asked for.
    pub scopes: Vec<String>,
}

/// A parser for one file, line after line.
pub(crate) struct LineParser {
    set: &'static SyntaxSet,
    state: ParseState,
    stack: ScopeStack,
    rules: HashMap<Scope, Option<(TokenClass, u8)>>,
    names: HashMap<Scope, Option<NameKind>>,
    collect_scopes: bool,
}

impl LineParser {
    /// A parser for the language, loading the syntax set if needed; `None`
    /// when the set has no grammar of that name.
    pub(crate) fn new(language: Language, collect_scopes: bool) -> Option<Self> {
        let set = syntax_set();
        let syntax = set.find_syntax_by_name(language.syntax_name())?;
        Some(Self {
            set,
            state: ParseState::new(syntax),
            stack: ScopeStack::new(),
            rules: HashMap::new(),
            names: HashMap::new(),
            collect_scopes,
        })
    }

    fn class_now(&mut self) -> TokenClass {
        let rules = &mut self.rules;
        let found: Vec<Option<(TokenClass, u8)>> = self
            .stack
            .as_slice()
            .iter()
            .rev()
            .map(|scope| {
                *rules
                    .entry(*scope)
                    .or_insert_with(|| rule_of(&scope.build_string()))
            })
            .collect();
        class_of(found)
    }

    fn name_now(&mut self) -> Option<NameKind> {
        let names = &mut self.names;
        self.stack.as_slice().iter().rev().find_map(|scope| {
            *names
                .entry(*scope)
                .or_insert_with(|| NameKind::of(&scope.build_string()))
        })
    }

    /// Tokenizes one line (without its line end); `None` when the grammar
    /// failed on it, after which the rest of the file is plain.
    pub(crate) fn line(&mut self, text: &str) -> Option<LineOut> {
        let fed = format!("{text}\n");
        let ops = self.state.parse_line(&fed, self.set).ok()?;
        let mut out = LineOut::default();
        let mut at = 0usize;
        let mut class = self.class_now();
        let mut name = self.name_now();
        for (pos, op) in ops {
            let pos = pos.min(text.len());
            if pos > at {
                emit(&mut out, at, pos, class, name);
                at = pos;
            }
            if self.collect_scopes
                && let ScopeStackOp::Push(scope) = &op
            {
                out.scopes.push(scope.build_string());
            }
            self.stack.apply(&op).ok()?;
            class = self.class_now();
            name = self.name_now();
        }
        if text.len() > at {
            emit(&mut out, at, text.len(), class, name);
        }
        Some(out)
    }
}

/// Adds a stretch of one class to the line, joined to the run before it
/// when the class is the same.
fn emit(out: &mut LineOut, start: usize, end: usize, class: TokenClass, name: Option<NameKind>) {
    let (start, end) = (start as u32, end as u32);
    if let Some(kind) = name {
        match out.names.last_mut() {
            Some(last) if last.end == start && last.kind == kind => last.end = end,
            _ => out.names.push(Named { start, end, kind }),
        }
    }
    if class == TokenClass::Plain {
        return;
    }
    match out.runs.last_mut() {
        Some(last) if last.end == start && last.class == class => last.end = end,
        _ => out.runs.push(ClassRun::new(start, end, class)),
    }
}
