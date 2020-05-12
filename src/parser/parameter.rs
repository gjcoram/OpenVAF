//  * ******************************************************************************************
//  * Copyright (c) 2019 Pascal Kuthe. This file is part of the VARF project.
//  * It is subject to the license terms in the LICENSE file found in the top-level directory
//  *  of this distribution and at  https://gitlab.com/DSPOM/VARF/blob/master/LICENSE.
//  *  No part of VARF, including this file, may be copied, modified, propagated, or
//  *  distributed except according to the terms contained in the LICENSE file.
//  * *******************************************************************************************

use std::ops::Range;

use copyless::VecHelper;

use crate::ast::{
    Expression, NumericalParameterRangeBound, NumericalParameterRangeExclude, Parameter,
    ParameterType, Primary, VariableType,
};
use crate::ir::{AttributeNode, Attributes, ExpressionId, Node, Push};
use crate::parser::error::Expected::ParameterRange;
use crate::parser::error::Result;
use crate::parser::error::Type::{UnexpectedToken, UnexpectedTokens, Unsupported};
use crate::parser::error::Unsupported::StringParameters;
use crate::parser::lexer::Token;
use crate::parser::Error;
use crate::symbol_table::SymbolDeclaration;
use crate::Parser;

impl<'lt, 'ast, 'source_map> Parser<'lt, 'ast, 'source_map> {
    pub fn parse_parameter_decl(&mut self, attributes: Attributes<'ast>) -> Result {
        let (token, span) = self.next()?;
        let parameter_type = match token {
            Token::Integer => VariableType::INTEGER,
            Token::Real => VariableType::REAL,
            Token::Realtime => VariableType::REALTIME,
            Token::Time => VariableType::TIME,
            Token::LiteralString => {
                return Err(Error {
                    error_type: Unsupported(StringParameters),
                    source: span,
                })
            }
            _ => {
                return Err(Error {
                    error_type: UnexpectedToken {
                        expected: vec![
                            Token::Integer,
                            Token::Real,
                            Token::Realtime,
                            Token::Time,
                            Token::LiteralString,
                        ],
                    },
                    source: span,
                })
            }
        };
        self.parse_list(
            |sel| sel.parse_numerical_parameter_assignment(attributes, parameter_type),
            Token::Semicolon,
            true,
        )?;
        Ok(())
    }

    fn parse_numerical_parameter_assignment(
        &mut self,
        attributes: Attributes<'ast>,
        base_type: VariableType,
    ) -> Result {
        let start = self.preprocessor.current_start();
        let name = self.parse_identifier(false)?;
        let default_value = if self.look_ahead()?.0 == Token::Assign {
            self.consume_lookahead();
            Some(self.parse_expression_id()?)
        } else {
            None
        };
        let mut included_ranges = Vec::new();
        let mut excluded_ranges = Vec::new();
        loop {
            match self.look_ahead()? {
                (Token::From, _) => {
                    self.consume_lookahead();
                    match self.next()? {
                        (Token::SquareBracketOpen, _) => {
                            included_ranges
                                .alloc()
                                .init(self.parse_parameter_range(true)?);
                        }
                        (Token::ParenOpen, _) => {
                            included_ranges
                                .alloc()
                                .init(self.parse_parameter_range(false)?);
                        }
                        (_, source) => {
                            return Err(Error {
                                error_type: UnexpectedTokens {
                                    expected: vec![ParameterRange],
                                },
                                source,
                            })
                        }
                    }
                }
                (Token::Exclude, _) => {
                    self.consume_lookahead();
                    match self.look_ahead()? {
                        (Token::SquareBracketOpen, _) => {
                            self.consume_lookahead();
                            excluded_ranges
                                .alloc()
                                .init(NumericalParameterRangeExclude::Range(
                                    self.parse_parameter_range(true)?,
                                ));
                        }
                        (Token::ParenOpen, _) => {
                            self.consume_lookahead();
                            excluded_ranges
                                .alloc()
                                .init(NumericalParameterRangeExclude::Range(
                                    self.parse_parameter_range(false)?,
                                ));
                        }
                        _ => {
                            excluded_ranges
                                .alloc()
                                .init(NumericalParameterRangeExclude::Value(
                                    self.parse_expression_id()?,
                                ));
                        }
                    }
                }
                (Token::Semicolon, _) | (Token::Comma, _) => break,
                (_, source) => {
                    return Err(Error {
                        error_type: UnexpectedToken {
                            expected: vec![Token::From, Token::Exclude, Token::Semicolon],
                        },
                        source,
                    })
                }
            }
        }
        let parameter_id = self.ast.push(AttributeNode {
            attributes,
            source: self.span_to_current_end(start),
            contents: Parameter {
                name,
                parameter_type: ParameterType::Numerical {
                    parameter_type: base_type,
                    included_ranges,
                    excluded_ranges,
                },
                default_value,
            },
        });
        self.insert_symbol(name, SymbolDeclaration::Parameter(parameter_id));
        Ok(())
    }

    fn parse_parameter_range(
        &mut self,
        inclusive: bool,
    ) -> Result<Range<NumericalParameterRangeBound<'ast>>> {
        let start = NumericalParameterRangeBound {
            bound: self.parse_parameter_range_expression()?,
            inclusive,
        };
        self.expect(Token::Colon)?;
        let end = self.parse_parameter_range_expression()?;
        let end_inclusive = match self.next()? {
            (Token::SquareBracketClose, _) => true,
            (Token::ParenClose, _) => false,
            (_, source) => {
                return Err(Error {
                    error_type: UnexpectedToken {
                        expected: vec![Token::ParenClose, Token::SquareBracketClose],
                    },
                    source,
                })
            }
        };
        Ok(start..NumericalParameterRangeBound {
            bound: end,
            inclusive: end_inclusive,
        })
    }

    fn parse_parameter_range_expression(&mut self) -> Result<ExpressionId<'ast>> {
        let (token, source) = self.look_ahead()?;
        match token {
            Token::Infinity => {
                self.consume_lookahead();
                Ok(self.ast.push(Node {
                    contents: Expression::Primary(Primary::Real(core::f64::INFINITY)),
                    source,
                }))
            }
            Token::MinusInfinity => {
                self.consume_lookahead();
                Ok(self.ast.push(Node {
                    contents: Expression::Primary(Primary::Real(core::f64::NEG_INFINITY)),
                    source,
                }))
            }
            _ => Ok(self.parse_expression_id()?),
        }
    }
}