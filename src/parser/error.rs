use annotate_snippets::display_list::DisplayList;
use annotate_snippets::formatter::DisplayListFormatter;
use annotate_snippets::snippet::{Annotation, AnnotationType, Slice, Snippet, SourceAnnotation};

use crate::{SourceMap, Span};
//  * ******************************************************************************************
//  * Copyright (c) 2019 Pascal Kuthe. This file is part of the VARF project.
//  * It is subject to the license terms in the LICENSE file found in the top-level directory
//  *  of this distribution and at  https://gitlab.com/jamescoding/VARF/blob/master/LICENSE.
//  *  No part of VARF, including this file, may be copied, modified, propagated, or
//  *  distributed except according to the terms contained in the LICENSE file.
//  * *******************************************************************************************
use crate::ast::{AstAttributeNodeId, AstNodeId, Module, Port};
use crate::parser::lexer::Token;
use crate::parser::preprocessor::ArgumentIndex;

pub type Error = crate::error::Error<Type>;
pub(crate) type Warning = crate::error::Error<WarningType>;
pub type Result<T = ()> = std::result::Result<T, Error>;
pub type MultiResult<T = ()> = std::result::Result<T, MultiError>;
pub struct MultiError(pub Vec<Error>);

impl MultiError {
    pub fn merge(mut self, mut other: Self) -> Self {
        self.0.append(&mut other.0);
        self
    }
    pub fn add(&mut self, err: Error) {
        self.0.push(err)
    }
}
impl From<Error> for MultiError {
    fn from(err: Error) -> Self {
        MultiError(vec![err])
    }
}

#[derive(Debug, Clone)]
pub enum Type {
    PortRedeclaration,
    EmptyListEntry(List),

    //Preprocessor
    MacroArgumentCount {
        expected: ArgumentIndex,
        found: ArgumentIndex,
    },
    ConditionEndWithoutStart,
    MacroEndTooEarly,
    UnclosedConditions(Vec<Span>),
    MacroNotFound,
    MacroRecursion,
    CompilerDirectiveSplit,
    IoErr(String),

    //General
    UnexpectedEof {
        expected: Vec<Token>,
    },
    UnexpectedToken {
        expected: Vec<Token>,
    },
    UnexpectedTokens {
        expected: Vec<Expected>,
    },

    Unsupported(Unsupported),
}
impl From<std::io::Error> for Type {
    fn from(io_err: std::io::Error) -> Self {
        Self::IoErr(io_err.to_string())
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Unsupported {
    DefaultDiscipline,
}
#[derive(Debug, Copy, Clone)]
pub enum Expected {
    Identifier,
    PortDeclaration,
    Port,
}
#[derive(Clone, Debug)]
pub(crate) enum WarningType {
    MacroOverwritten(Span),
}

#[derive(Debug, Clone, Copy)]
pub enum List {
    MacroArgument,
    FunctionArgument,
    Identifier,
}
impl Error {
    pub fn print(&self, source_map: &SourceMap) {
        let snippet = match self.error_type {
            Type::UnexpectedToken { ref expected } => {
                let (line, line_number, range) = source_map.resolve_span_within_line(self.source);
                Snippet {
                    title: Some(Annotation {
                        id: None,
                        label: Some("Unexpected Token".to_string()),
                        annotation_type: AnnotationType::Error,
                    }),
                    footer: vec![],
                    slices: vec![Slice {
                        source: line,
                        line_start: line_number as usize,
                        origin: None,
                        annotations: vec![SourceAnnotation {
                            range: (range.start, range.end),
                            label: format!("expected {:?}", expected),
                            annotation_type: AnnotationType::Error,
                        }],
                        fold: false,
                    }],
                }
            }
            _ => unimplemented!("{:?}", self.error_type),
        };
        let display_list = DisplayList::from(snippet);
        let formatter = DisplayListFormatter::new(true, false);
        formatter.format(&display_list);
    }
}
pub fn merge_multi_result<T>(value_res: MultiResult<T>, condition: MultiResult) -> MultiResult<T> {
    match value_res {
        Ok(res) => {
            condition?;
            Ok(res)
        }
        Err(res) => {
            if let Err(err) = condition {
                Err(res.merge(err))
            } else {
                Err(res)
            }
        }
    }
}
pub fn into_multi_res<T>(res: Result<T>) -> MultiResult<T> {
    res.map_err(|err| err.into())
}
pub fn add_error<T>(res: MultiResult<T>, error: Error) -> MultiResult<T> {
    match res {
        Ok(_) => Err(error.into()),
        Err(mut res) => {
            res.add(error);
            Err(res)
        }
    }
}