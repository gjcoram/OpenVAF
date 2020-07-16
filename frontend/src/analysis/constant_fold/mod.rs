//  * ******************************************************************************************
//  * Copyright (c) 2020 Pascal Kuthe. This file is part of the frontend project.
//  * It is subject to the license terms in the LICENSE file found in the top-level directory
//  *  of this distribution and at  https://gitlab.com/DSPOM/OpenVAF/blob/master/LICENSE.
//  *  No part of frontend, including this file, may be copied, modified, propagated, or
//  *  distributed except according to the terms contained in the LICENSE file.
//  * *******************************************************************************************

//! Simple constant folding algorithm

use core::mem::replace;
use core::ops::{Add, Div, Mul, Sub};
use core::option::Option::Some;

use num_traits::{One, Pow, Zero};

pub use lints::ConstantOverflow;
pub use propagation::PropagatedConstants;
pub use resolver::{ConstResolver, ConstantPropagator, NoConstResolution};

use crate::ast::UnaryOperator;
use crate::ir::hir::DisciplineAccess;
use crate::ir::mir::fold::integer_expressions::{
    IntegerBinaryOperatorFold, IntegerComparisonFold, IntegerExprFold, RealComparisonFold,
};
use crate::ir::mir::fold::real_expressions::{
    RealBinaryOperatorFold, RealBuiltInFunctionCall2pFold, RealExprFold,
};
use crate::ir::mir::fold::string_expressions::StringExprFold;
use crate::ir::mir::Mir;
use crate::ir::{
    BranchId, BuiltInFunctionCall1p, BuiltInFunctionCall2p, IntegerExpressionId, NetId, Node,
    NoiseSource, ParameterId, PortId, RealExpressionId, StringExpressionId, VariableId,
};
use crate::lints::Linter;
use crate::literals::StringLiteral;
use crate::mir::fold::integer_expressions::walk_integer_expression;
use crate::mir::fold::real_expressions::{walk_real_expression, RealBuiltInFunctionCall1pFold};
use crate::mir::fold::string_expressions::walk_string_expression;
use crate::mir::{
    ComparisonOperator, IntegerBinaryOperator, IntegerExpression, RealBinaryOperator,
    RealExpression, StringExpression,
};
use crate::sourcemap::Span;
use float_cmp::{ApproxEq, F64Margin};
use log::trace;

mod lints;
mod propagation;
mod resolver;

/// Abstraction over mutability of the mir for constant folding (see [`constant_eval_real_expr`](crate::mir::Mir::constant_eval_real_expr))
trait ConstantFoldType: AsRef<Mir> {
    fn overwrite_real_expr(&mut self, expr: RealExpressionId, val: RealExpression);
    fn overwrite_int_expr(&mut self, expr: IntegerExpressionId, val: IntegerExpression);
    fn overwrite_str_expr(&mut self, expr: StringExpressionId, val: StringExpression);
}

struct MutatingConstantFold<'lt>(pub &'lt mut Mir);

impl<'lt> AsRef<Mir> for MutatingConstantFold<'lt> {
    fn as_ref(&self) -> &Mir {
        &self.0
    }
}

impl<'lt> ConstantFoldType for MutatingConstantFold<'lt> {
    fn overwrite_real_expr(&mut self, expr: RealExpressionId, val: RealExpression) {
        self.0[expr].contents = val;
    }

    fn overwrite_int_expr(&mut self, expr: IntegerExpressionId, val: IntegerExpression) {
        self.0[expr].contents = val;
    }

    fn overwrite_str_expr(&mut self, expr: StringExpressionId, val: StringExpression) {
        self.0[expr].contents = val;
    }
}

struct ReadingConstantFold<'lt>(pub &'lt Mir);

impl<'lt> AsRef<Mir> for ReadingConstantFold<'lt> {
    fn as_ref(&self) -> &Mir {
        &self.0
    }
}

impl<'lt> ConstantFoldType for ReadingConstantFold<'lt> {
    #[inline(always)]
    fn overwrite_real_expr(&mut self, _: RealExpressionId, _: RealExpression) {}

    #[inline(always)]
    fn overwrite_int_expr(&mut self, _: IntegerExpressionId, _: IntegerExpression) {}

    #[inline(always)]
    fn overwrite_str_expr(&mut self, _: StringExpressionId, _: StringExpression) {}
}

enum ArgSide {
    Rhs,
    Lhs,
}

/// Implementation of constant folding as a [mir expression fold](crate::ir::mir::fold).
/// All methods return `None` if constant folding was not possible and `Some(value)` otherwise

struct ConstantFold<'lt, T: ConstantFoldType, R: ConstResolver, E> {
    fold_type: &'lt mut T,
    resolver: &'lt mut R,
    expr: E,
}

impl<'lt, T: ConstantFoldType, R: ConstResolver, E> ConstantFold<'lt, T, R, E> {
    fn fold_real_expression(&mut self, expr: RealExpressionId) -> Option<f64> {
        ConstantFold {
            fold_type: self.fold_type,
            resolver: self.resolver,
            expr,
        }
        .fold_real_expr(expr)
    }

    fn fold_int_expression(&mut self, expr: IntegerExpressionId) -> Option<i64> {
        ConstantFold {
            fold_type: self.fold_type,
            resolver: self.resolver,
            expr,
        }
        .fold_integer_expr(expr)
    }

    fn fold_str_expression(&mut self, expr: StringExpressionId) -> Option<StringLiteral> {
        ConstantFold {
            fold_type: self.fold_type,
            resolver: self.resolver,
            expr,
        }
        .fold_string_expr(expr)
    }
}

impl<'lt, T: ConstantFoldType + 'lt, R: ConstResolver + 'lt, E> ConstantFold<'lt, T, R, E> {
    fn fold_pow<EXP: One + Zero + PartialEq, B: One + Zero + Pow<EXP, Output = B> + PartialEq>(
        arg1: Option<B>,
        arg2: Option<EXP>,
        resolve_to: impl FnOnce(ArgSide),
    ) -> Option<B> {
        match (arg1, arg2) {
            (_, Some(exponent)) if exponent.is_zero() => Some(B::zero()),
            (None, Some(exponent)) if exponent.is_one() => {
                resolve_to(ArgSide::Lhs);
                None
            }

            (Some(base), _) if base.is_zero() => Some(B::zero()),

            (Some(base), _) if base.is_one() => Some(B::one()),

            (Some(arg1), Some(arg2)) => Some(arg1.pow(arg2)),

            _ => None,
        }
    }

    fn fold_mul<V: One + Zero + Mul<Output = V> + PartialEq>(
        arg1: Option<V>,
        arg2: Option<V>,
        resolve_to: impl FnOnce(ArgSide),
    ) -> Option<V> {
        match (arg1, arg2) {
            (None, Some(arg)) | (Some(arg), None) if arg.is_zero() => Some(V::zero()),

            (None, Some(one)) if one.is_one() => {
                resolve_to(ArgSide::Lhs);
                None
            }

            (Some(one), None) if one.is_one() => {
                resolve_to(ArgSide::Rhs);
                None
            }

            (Some(arg1), Some(arg2)) => Some(arg1 * arg2),

            _ => None,
        }
    }

    fn fold_div<V: One + Zero + Div<Output = V> + PartialEq>(
        arg1: Option<V>,
        arg2: Option<V>,
        resolve_to: impl FnOnce(ArgSide),
    ) -> Option<V> {
        match (arg1, arg2) {
            (Some(lhs), None) if lhs.is_zero() => Some(V::zero()),

            (None, Some(rhs)) if rhs.is_one() => {
                resolve_to(ArgSide::Lhs);
                None
            }

            (Some(arg1), Some(arg2)) => Some(arg1 / arg2),

            _ => None,
        }
    }

    fn fold_plus<V: Zero + Add>(
        arg1: Option<V>,
        arg2: Option<V>,
        resolve_to: impl FnOnce(ArgSide),
    ) -> Option<V> {
        match (arg1, arg2) {
            (None, Some(rhs)) if rhs.is_zero() => {
                resolve_to(ArgSide::Lhs);
                None
            }

            (Some(lhs), None) if lhs.is_zero() => {
                resolve_to(ArgSide::Rhs);
                None
            }

            (Some(arg1), Some(arg2)) => Some(arg1 + arg2),

            _ => None,
        }
    }
    fn fold_minus<V: Zero + Add + Sub<V, Output = V> + PartialEq>(
        arg1: Option<V>,
        arg2: Option<V>,
        resolve_to: impl FnOnce(ArgSide),
    ) -> Option<V> {
        match (arg1, arg2) {
            (None, Some(rhs)) if rhs.is_zero() => {
                resolve_to(ArgSide::Lhs);
                None
            }

            (Some(arg1), Some(arg2)) => Some(arg1 - arg2),

            _ => None,
        }
    }
}
impl<'lt, T: ConstantFoldType, R: ConstResolver> ConstantFold<'lt, T, R, RealExpressionId> {
    fn real_resolve_to(&mut self, lhs: RealExpressionId, rhs: RealExpressionId, side: ArgSide) {
        let val = match side {
            ArgSide::Lhs => lhs,
            ArgSide::Rhs => rhs,
        };
        let val = self.mir()[val].contents.clone();
        self.fold_type.overwrite_real_expr(self.expr, val)
    }
}

impl<'lt, T: ConstantFoldType, R: ConstResolver> ConstantFold<'lt, T, R, IntegerExpressionId> {
    fn int_resolve_to(
        &mut self,
        lhs: IntegerExpressionId,
        rhs: IntegerExpressionId,
        side: ArgSide,
    ) {
        let val = match side {
            ArgSide::Lhs => lhs,
            ArgSide::Rhs => rhs,
        };
        let val = self.mir()[val].contents.clone();
        self.fold_type.overwrite_int_expr(self.expr, val)
    }
}

impl<'lt, T: ConstantFoldType, R: ConstResolver> RealExprFold
    for ConstantFold<'lt, T, R, RealExpressionId>
{
    type T = Option<f64>;

    fn mir(&self) -> &Mir {
        self.fold_type.as_ref()
    }

    #[inline]
    fn fold_real_expr(&mut self, expr: RealExpressionId) -> Option<f64> {
        let old = replace(&mut self.expr, expr);
        let res = walk_real_expression(self, expr);
        self.expr = old;
        res
    }

    fn fold_literal(&mut self, val: f64) -> Option<f64> {
        Some(val)
    }

    fn fold_binary_operator(
        &mut self,
        lhs: RealExpressionId,
        op: Node<RealBinaryOperator>,
        rhs: RealExpressionId,
    ) -> Option<f64> {
        self.fold_real_binary_op(lhs, op.contents, rhs)
    }

    fn fold_negate(&mut self, _: Span, arg: RealExpressionId) -> Option<f64> {
        Some(-self.fold_real_expr(arg)?)
    }

    fn fold_condition(
        &mut self,
        cond: IntegerExpressionId,
        true_expr: RealExpressionId,
        false_expr: RealExpressionId,
    ) -> Option<f64> {
        let cond = self.fold_int_expression(cond)?;

        let (expr, res) = if cond == 0 {
            (false_expr, self.fold_real_expr(false_expr))
        } else {
            (true_expr, self.fold_real_expr(true_expr))
        };

        if res.is_none() {
            self.fold_type
                .overwrite_real_expr(self.expr, self.mir()[expr].contents.clone())
        }

        res
    }

    fn fold_variable_reference(&mut self, var: VariableId) -> Option<f64> {
        self.resolver.real_variable_value(var)
    }

    fn fold_parameter_reference(&mut self, param: ParameterId) -> Option<f64> {
        self.resolver.real_parameter_value(param)
    }

    fn fold_branch_access(
        &mut self,
        _discipline_accesss: DisciplineAccess,
        _branch: BranchId,
        _time_derivative_order: u8,
    ) -> Option<f64> {
        None
    }

    fn fold_noise(
        &mut self,
        _noise_src: NoiseSource<RealExpressionId, ()>,
        _name: Option<StringLiteral>,
    ) -> Option<f64> {
        None
    }

    fn fold_builtin_function_call_1p(
        &mut self,
        call: BuiltInFunctionCall1p,
        arg: RealExpressionId,
    ) -> Option<f64> {
        self.fold_real_builtin_function_call_1p(call, arg)
    }

    fn fold_builtin_function_call_2p(
        &mut self,
        call: BuiltInFunctionCall2p,
        arg1: RealExpressionId,
        arg2: RealExpressionId,
    ) -> Option<f64> {
        self.fold_real_builtin_function_call_2p(call, arg1, arg2)
    }

    fn fold_temperature(&mut self) -> Option<f64> {
        None
    }

    fn fold_sim_param(
        &mut self,
        _name: StringExpressionId,
        _default: Option<RealExpressionId>,
    ) -> Option<f64> {
        None
    }

    fn fold_integer_conversion(&mut self, expr: IntegerExpressionId) -> Option<f64> {
        let val = ConstantFold {
            expr,
            fold_type: self.fold_type,
            resolver: self.resolver,
        }
        .fold_integer_expr(expr)? as f64;
        Some(val)
    }
}

impl<'lt, T: ConstantFoldType, R: ConstResolver> RealBuiltInFunctionCall2pFold
    for ConstantFold<'lt, T, R, RealExpressionId>
{
    type T = Option<f64>;

    fn fold_pow(
        &mut self,
        arg1_expr: RealExpressionId,
        arg2_expr: RealExpressionId,
    ) -> Option<f64> {
        let arg1 = self.fold_real_expr(arg1_expr);
        let arg2 = self.fold_real_expr(arg2_expr);
        Self::fold_pow(arg1, arg2, |side| {
            self.real_resolve_to(arg1_expr, arg2_expr, side)
        })
    }

    fn fold_hypot(&mut self, arg1: RealExpressionId, arg2: RealExpressionId) -> Option<f64> {
        let arg1 = self.fold_real_expr(arg1)?;
        let arg2 = self.fold_real_expr(arg2)?;
        Some(arg1.hypot(arg2))
    }

    fn fold_arctan2(&mut self, arg1: RealExpressionId, arg2: RealExpressionId) -> Option<f64> {
        let arg1 = self.fold_real_expr(arg1)?;
        let arg2 = self.fold_real_expr(arg2)?;
        Some(arg1.atan2(arg2))
    }

    fn fold_max(&mut self, arg1: RealExpressionId, arg2: RealExpressionId) -> Option<f64> {
        let arg1 = self.fold_real_expr(arg1)?;
        let arg2 = self.fold_real_expr(arg2)?;
        Some(arg1.max(arg2))
    }

    fn fold_min(&mut self, arg1: RealExpressionId, arg2: RealExpressionId) -> Option<f64> {
        let arg1 = self.fold_real_expr(arg1)?;
        let arg2 = self.fold_real_expr(arg2)?;
        Some(arg1.min(arg2))
    }
}

impl<'lt, T: ConstantFoldType, R: ConstResolver> RealBuiltInFunctionCall1pFold
    for ConstantFold<'lt, T, R, RealExpressionId>
{
    type T = Option<f64>;

    fn fold_sqrt(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.sqrt())
    }

    fn fold_exp(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.exp())
    }

    fn fold_ln(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.ln())
    }

    fn fold_log(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.log10())
    }

    fn fold_abs(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.abs())
    }

    fn fold_floor(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.floor())
    }

    fn fold_ceil(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.ceil())
    }

    fn fold_sin(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.sin())
    }

    fn fold_cos(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.cos())
    }

    fn fold_tan(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.tan())
    }

    fn fold_arcsin(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.asin())
    }

    fn fold_arccos(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.acos())
    }

    fn fold_arctan(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.atan())
    }

    fn fold_sinh(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.sinh())
    }

    fn fold_cosh(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.cosh())
    }

    fn fold_tanh(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.tanh())
    }

    fn fold_arcsinh(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.asinh())
    }

    fn fold_arccosh(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.acosh())
    }

    fn fold_arctanh(&mut self, arg: RealExpressionId) -> Option<f64> {
        self.fold_real_expr(arg).map(|arg| arg.atanh())
    }
}

impl<'lt, T: ConstantFoldType, R: ConstResolver> RealBinaryOperatorFold
    for ConstantFold<'lt, T, R, RealExpressionId>
{
    type T = Option<f64>;

    fn fold_sum(&mut self, lhs_expr: RealExpressionId, rhs_expr: RealExpressionId) -> Option<f64> {
        let lhs = self.fold_real_expr(lhs_expr);
        let rhs = self.fold_real_expr(rhs_expr);
        Self::fold_plus(lhs, rhs, |side| {
            self.real_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_diff(&mut self, lhs_expr: RealExpressionId, rhs_expr: RealExpressionId) -> Option<f64> {
        let lhs = self.fold_real_expr(lhs_expr);
        let rhs = self.fold_real_expr(rhs_expr);
        Self::fold_minus(lhs, rhs, |side| {
            self.real_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_mul(&mut self, lhs_expr: RealExpressionId, rhs_expr: RealExpressionId) -> Option<f64> {
        let lhs = self.fold_real_expr(lhs_expr);
        let rhs = self.fold_real_expr(rhs_expr);

        Self::fold_mul(lhs, rhs, |side| {
            self.real_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_quotient(
        &mut self,
        lhs_expr: RealExpressionId,
        rhs_expr: RealExpressionId,
    ) -> Option<f64> {
        let lhs = self.fold_real_expr(lhs_expr);
        let rhs = self.fold_real_expr(rhs_expr);
        Self::fold_div(lhs, rhs, |side| {
            self.real_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_pow(&mut self, lhs_expr: RealExpressionId, rhs_expr: RealExpressionId) -> Option<f64> {
        let lhs = self.fold_real_expr(lhs_expr);
        let rhs = self.fold_real_expr(rhs_expr);
        Self::fold_pow(lhs, rhs, |side| {
            self.real_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_mod(&mut self, lhs: RealExpressionId, rhs: RealExpressionId) -> Option<f64> {
        let lhs = self.fold_real_expr(lhs)?;
        let rhs = self.fold_real_expr(rhs)?;
        Some(lhs % rhs)
    }
}

impl<'lt, T: ConstantFoldType, R: ConstResolver> IntegerExprFold
    for ConstantFold<'lt, T, R, IntegerExpressionId>
{
    type T = Option<i64>;

    fn mir(&self) -> &Mir {
        self.fold_type.as_ref()
    }
    fn fold_integer_expr(&mut self, expr: IntegerExpressionId) -> Option<i64> {
        let old = replace(&mut self.expr, expr);
        let res = walk_integer_expression(self, expr);
        if let Some(res) = res {
            self.fold_type
                .overwrite_int_expr(self.expr, IntegerExpression::Literal(res))
        }
        self.expr = old;
        res
    }
    fn fold_literal(&mut self, val: i64) -> Option<i64> {
        Some(val)
    }

    fn fold_binary_operator(
        &mut self,
        lhs: IntegerExpressionId,
        op: Node<IntegerBinaryOperator>,
        rhs: IntegerExpressionId,
    ) -> Option<i64> {
        self.fold_integer_binary_op(lhs, op.contents, rhs)
    }

    fn fold_integer_comparison(
        &mut self,
        lhs: IntegerExpressionId,
        op: Node<ComparisonOperator>,
        rhs: IntegerExpressionId,
    ) -> Option<i64> {
        IntegerComparisonFold::fold_integer_comparison(self, lhs, op.contents, rhs)
    }

    fn fold_real_comparison(
        &mut self,
        lhs: RealExpressionId,
        op: Node<ComparisonOperator>,
        rhs: RealExpressionId,
    ) -> Option<i64> {
        RealComparisonFold::fold_real_comparison(self, lhs, op.contents, rhs)
    }

    fn fold_unary_op(&mut self, op: Node<UnaryOperator>, arg: IntegerExpressionId) -> Option<i64> {
        let arg = self.fold_integer_expr(arg)?;
        let res = match op.contents {
            UnaryOperator::BitNegate | UnaryOperator::LogicNegate => !arg,
            UnaryOperator::ArithmeticNegate => -arg,
            UnaryOperator::ExplicitPositive => arg,
        };
        Some(res)
    }

    fn fold_condition(
        &mut self,
        cond: IntegerExpressionId,
        true_expr: IntegerExpressionId,
        false_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let cond = self.fold_int_expression(cond)?;

        let (expr, res) = if cond == 0 {
            (false_expr, self.fold_integer_expr(false_expr))
        } else {
            (true_expr, self.fold_integer_expr(true_expr))
        };

        if res.is_none() {
            self.fold_type
                .overwrite_int_expr(self.expr, self.mir()[expr].contents.clone())
        }

        res
    }

    fn fold_min(&mut self, arg1: IntegerExpressionId, arg2: IntegerExpressionId) -> Option<i64> {
        let arg1 = self.fold_integer_expr(arg1)?;
        let arg2 = self.fold_integer_expr(arg2)?;
        Some(arg1.min(arg2))
    }

    fn fold_max(&mut self, arg1: IntegerExpressionId, arg2: IntegerExpressionId) -> Option<i64> {
        let arg1 = self.fold_integer_expr(arg1)?;
        let arg2 = self.fold_integer_expr(arg2)?;
        Some(arg1.max(arg2))
    }

    fn fold_abs(&mut self, arg: IntegerExpressionId) -> Option<i64> {
        let arg = self.fold_integer_expr(arg)?;
        Some(arg.abs())
    }

    fn fold_variable_reference(&mut self, var: VariableId) -> Option<i64> {
        self.resolver.int_variable_value(var)
    }

    fn fold_parameter_reference(&mut self, param: ParameterId) -> Option<i64> {
        self.resolver.int_parameter_value(param)
    }

    fn fold_real_cast(&mut self, expr: RealExpressionId) -> Option<i64> {
        let res = self.fold_real_expression(expr)?.round();

        Some(res as i64)
    }

    fn fold_port_connected(&mut self, _: PortId) -> Option<i64> {
        None
    }

    fn fold_param_given(&mut self, _: ParameterId) -> Option<i64> {
        None
    }

    fn fold_port_reference(&mut self, _: PortId) -> Option<i64> {
        None
    }

    fn fold_net_reference(&mut self, _: NetId) -> Option<i64> {
        None
    }

    fn fold_string_eq(&mut self, lhs: StringExpressionId, rhs: StringExpressionId) -> Option<i64> {
        let lhs = self.fold_str_expression(lhs)?;
        let rhs = self.fold_str_expression(rhs)?;

        Some((rhs == lhs) as i64)
    }

    fn fold_string_neq(&mut self, lhs: StringExpressionId, rhs: StringExpressionId) -> Option<i64> {
        let lhs = self.fold_str_expression(lhs)?;
        let rhs = self.fold_str_expression(rhs)?;

        Some((rhs != lhs) as i64)
    }
}

const U32_MAX_I64: i64 = u32::MAX as i64;
const U32_OVERFLOW_START: i64 = U32_MAX_I64 + 1;

impl<'lt, T: ConstantFoldType, R: ConstResolver> IntegerBinaryOperatorFold
    for ConstantFold<'lt, T, R, IntegerExpressionId>
{
    type T = Option<i64>;

    fn fold_sum(
        &mut self,
        lhs_expr: IntegerExpressionId,
        rhs_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs_expr);
        let rhs = self.fold_integer_expr(rhs_expr);

        Self::fold_plus(lhs, rhs, |side| {
            self.int_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_diff(
        &mut self,
        lhs_expr: IntegerExpressionId,
        rhs_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs_expr);
        let rhs = self.fold_integer_expr(rhs_expr);

        Self::fold_minus(lhs, rhs, |side| {
            self.int_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_mul(
        &mut self,
        lhs_expr: IntegerExpressionId,
        rhs_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs_expr);
        let rhs = self.fold_integer_expr(rhs_expr);

        Self::fold_mul(lhs, rhs, |side| {
            self.int_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_quotient(
        &mut self,
        lhs_expr: IntegerExpressionId,
        rhs_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs_expr);
        let rhs = self.fold_integer_expr(rhs_expr);

        if rhs == Some(0) {
            Linter::dispatch_late(
                Box::new(ConstantOverflow(self.mir()[self.expr].span)),
                self.expr.into(),
            )
        }

        Self::fold_div(lhs, rhs, |side| {
            self.int_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_pow(
        &mut self,
        lhs_expr: IntegerExpressionId,
        rhs_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs_expr);
        let rhs = match self.fold_integer_expr(rhs_expr) {
            // Negative powers are 0 < |1/x| < 1 and as such always truncated to 0 according to VAMS standard
            Some(i64::MIN..=-1) => return Some(0),

            // valid range
            Some(val @ 0..=U32_MAX_I64) => Some(val as u32),

            // raising to a power higher than u32::MAX leads to an overflow
            Some(U32_OVERFLOW_START..=i64::MAX) => {
                Linter::dispatch_late(
                    Box::new(ConstantOverflow(self.mir()[self.expr].span)),
                    self.expr.into(),
                );
                None
            }

            None => None,
        };

        Self::fold_pow(lhs, rhs, |side| {
            self.int_resolve_to(lhs_expr, rhs_expr, side)
        })
    }

    fn fold_mod(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;

        Some(lhs % rhs)
    }

    fn fold_shiftl(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;

        Some(lhs << rhs)
    }

    fn fold_shiftr(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;

        Some(lhs >> rhs)
    }

    fn fold_xor(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;

        Some(lhs ^ rhs)
    }

    fn fold_nxor(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;

        Some(!(lhs ^ rhs))
    }

    fn fold_and(
        &mut self,
        lhs_expr: IntegerExpressionId,
        rhs_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs_expr);
        let rhs = self.fold_integer_expr(rhs_expr);
        trace!("folding {:?} & {:?}", lhs, rhs);

        match (lhs, rhs) {
            (Some(0), _) | (_, Some(0)) => Some(0),
            (Some(i64::MAX), None) => {
                self.fold_type
                    .overwrite_int_expr(self.expr, self.mir()[lhs_expr].contents.clone());
                None
            }
            (None, Some(i64::MAX)) => {
                self.fold_type
                    .overwrite_int_expr(self.expr, self.mir()[rhs_expr].contents.clone());
                None
            }
            (Some(lhs), Some(rhs)) => Some(lhs & rhs),
            _ => None,
        }
    }

    fn fold_or(
        &mut self,
        lhs_expr: IntegerExpressionId,
        rhs_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs_expr);
        let rhs = self.fold_integer_expr(rhs_expr);
        trace!("folding {:?} | {:?}", lhs, rhs);

        match (lhs, rhs) {
            (Some(i64::MAX), _) | (_, Some(i64::MAX)) => Some(i64::MAX),
            (Some(0), None) => {
                self.fold_type
                    .overwrite_int_expr(self.expr, self.mir()[lhs_expr].contents.clone());
                None
            }
            (None, Some(0)) => {
                self.fold_type
                    .overwrite_int_expr(self.expr, self.mir()[rhs_expr].contents.clone());
                None
            }
            (Some(lhs), Some(rhs)) => Some(lhs | rhs),
            _ => None,
        }
    }

    fn fold_logic_and(
        &mut self,
        lhs_expr: IntegerExpressionId,
        rhs_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs_expr);
        let rhs = self.fold_integer_expr(rhs_expr);
        trace!("folding {:?} && {:?}", lhs, rhs);

        match (lhs, rhs) {
            (Some(x), _) | (_, Some(x)) if x == 0 => Some(0),
            (Some(x), None) if x != 0 => {
                self.fold_type
                    .overwrite_int_expr(self.expr, self.mir()[lhs_expr].contents.clone());
                None
            }
            (None, Some(x)) if x != 0 => {
                self.fold_type
                    .overwrite_int_expr(self.expr, self.mir()[rhs_expr].contents.clone());
                None
            }
            (Some(lhs), Some(rhs)) => Some(((lhs != 0) && (rhs != 0)) as i64),
            _ => None,
        }
    }

    fn fold_logic_or(
        &mut self,
        lhs_expr: IntegerExpressionId,
        rhs_expr: IntegerExpressionId,
    ) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs_expr);
        let rhs = self.fold_integer_expr(rhs_expr);
        trace!("folding {:?} || {:?}", lhs, rhs);

        match (lhs, rhs) {
            (Some(x), _) | (_, Some(x)) if x != 0 => Some(1),
            (Some(x), None) if x == 0 => {
                self.fold_type
                    .overwrite_int_expr(self.expr, self.mir()[lhs_expr].contents.clone());
                None
            }

            (None, Some(x)) if x == 0 => {
                self.fold_type
                    .overwrite_int_expr(self.expr, self.mir()[rhs_expr].contents.clone());
                None
            }

            (Some(lhs), Some(rhs)) => Some(((lhs != 0) || (rhs != 0)) as i64),
            _ => None,
        }
    }
}

impl<'lt, T: ConstantFoldType, R: ConstResolver> IntegerComparisonFold
    for ConstantFold<'lt, T, R, IntegerExpressionId>
{
    type T = Option<i64>;

    fn fold_lt(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;
        trace!("folding {:?} < {:?}", lhs, rhs);
        Some((lhs < rhs) as i64)
    }

    fn fold_le(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;
        trace!("folding {:?} <= {:?}", lhs, rhs);
        Some((lhs <= rhs) as i64)
    }

    fn fold_gt(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;
        trace!("folding {:?} > {:?}", lhs, rhs);
        Some((lhs > rhs) as i64)
    }

    fn fold_ge(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;
        trace!("folding {:?} >= {:?}", lhs, rhs);
        Some((lhs >= rhs) as i64)
    }

    fn fold_eq(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;
        trace!("folding {:?} == {:?}", lhs, rhs);
        Some((lhs == rhs) as i64)
    }

    fn fold_ne(&mut self, lhs: IntegerExpressionId, rhs: IntegerExpressionId) -> Option<i64> {
        let lhs = self.fold_integer_expr(lhs)?;
        let rhs = self.fold_integer_expr(rhs)?;
        trace!("folding {:?} != {:?}", lhs, rhs);
        Some((lhs != rhs) as i64)
    }
}

impl<'lt, T: ConstantFoldType, R: ConstResolver> RealComparisonFold
    for ConstantFold<'lt, T, R, IntegerExpressionId>
{
    type T = Option<i64>;

    fn fold_lt(&mut self, lhs: RealExpressionId, rhs: RealExpressionId) -> Option<i64> {
        let lhs = self.fold_real_expression(lhs)?;
        let rhs = self.fold_real_expression(rhs)?;
        trace!("folding {:?} < {:?}", lhs, rhs);
        Some((lhs < rhs) as i64)
    }

    fn fold_le(&mut self, lhs: RealExpressionId, rhs: RealExpressionId) -> Option<i64> {
        let lhs = self.fold_real_expression(lhs)?;
        let rhs = self.fold_real_expression(rhs)?;
        trace!("folding {:?} <= {:?}", lhs, rhs);
        Some((lhs <= rhs) as i64)
    }

    fn fold_gt(&mut self, lhs: RealExpressionId, rhs: RealExpressionId) -> Option<i64> {
        let lhs = self.fold_real_expression(lhs)?;
        let rhs = self.fold_real_expression(rhs)?;
        trace!("folding {:?} > {:?}", lhs, rhs);
        Some((lhs > rhs) as i64)
    }

    fn fold_ge(&mut self, lhs: RealExpressionId, rhs: RealExpressionId) -> Option<i64> {
        let lhs = self.fold_real_expression(lhs)?;
        let rhs = self.fold_real_expression(rhs)?;
        trace!("folding {:?} >= {:?}", lhs, rhs);
        Some((lhs >= rhs) as i64)
    }

    fn fold_eq(&mut self, lhs: RealExpressionId, rhs: RealExpressionId) -> Option<i64> {
        let lhs = self.fold_real_expression(lhs)?;
        let rhs = self.fold_real_expression(rhs)?;
        trace!("folding {:?} == {:?}", lhs, rhs);
        Some((lhs == rhs) as i64)
    }

    fn fold_ne(&mut self, lhs: RealExpressionId, rhs: RealExpressionId) -> Option<i64> {
        let lhs = self.fold_real_expression(lhs)?;
        let rhs = self.fold_real_expression(rhs)?;
        trace!("folding {:?} != {:?}", lhs, rhs);
        Some((lhs.approx_ne(rhs, F64Margin::default())) as i64)
    }
}

impl<'lt, T: ConstantFoldType, R: ConstResolver> StringExprFold
    for ConstantFold<'lt, T, R, StringExpressionId>
{
    type T = Option<StringLiteral>;

    #[inline]
    fn fold_string_expr(&mut self, expr: StringExpressionId) -> Option<StringLiteral> {
        let old = replace(&mut self.expr, expr);
        let res = walk_string_expression(self, expr);
        if let Some(res) = res {
            self.fold_type
                .overwrite_str_expr(expr, StringExpression::Literal(res))
        }
        self.expr = old;
        res
    }

    fn mir(&self) -> &Mir {
        self.fold_type.as_ref()
    }

    fn fold_literal(&mut self, val: StringLiteral) -> Option<StringLiteral> {
        Some(val)
    }

    fn fold_condition(
        &mut self,
        cond: IntegerExpressionId,
        true_expr: StringExpressionId,
        false_expr: StringExpressionId,
    ) -> Option<StringLiteral> {
        let cond = self.fold_int_expression(cond)?;
        let (expr, val) = if cond == 0 {
            (false_expr, self.fold_string_expr(false_expr))
        } else {
            (true_expr, self.fold_string_expr(true_expr))
        };

        if val.is_none() {
            self.fold_type
                .overwrite_str_expr(self.expr, self.mir()[expr].contents.clone())
        }

        val
    }

    fn fold_variable_reference(&mut self, var: VariableId) -> Option<StringLiteral> {
        self.resolver.str_variable_value(var)
    }

    fn fold_parameter_reference(&mut self, param: ParameterId) -> Option<StringLiteral> {
        self.resolver.str_parameter_value(param)
    }

    fn fold_sim_parameter(&mut self, _name: StringExpressionId) -> Option<StringLiteral> {
        None
    }
}

impl Mir {
    /// Tries to fold as many constants in `expr` as possible.
    ///
    /// This function can only fold simple constants
    /// (for example `abs(x) > 0?0:1` can not be cosntant folded currently)
    ///
    ///
    /// # Arguments
    ///
    /// * `expr` - the Expression to be constant folded
    /// * `resolver` - Trait that handles constant folding references to variables and parameters
    ///
    ///
    /// # Returns
    ///
    /// * `None` if `expr` was only partially (or not at all) constant folded
    /// * `Some(val)` if `expr` was completly constant folded
    ///
    ///
    /// # Examples
    ///
    /// `x + (32*2)`
    /// * `resolver` can not resolve `foo`: `foo + (32*2)` -> `foo + 64` (return value `None`)
    /// * `resolver` resolved `foo` to `-16`: `foo + (32*2)` -> `foo + 64` (return value `Some(42)`)
    ///
    pub fn constant_fold_real_expr(
        &mut self,
        expr: RealExpressionId,
        resolver: &mut impl ConstResolver,
    ) -> Option<f64> {
        ConstantFold {
            fold_type: &mut MutatingConstantFold(self),
            resolver,
            expr,
        }
        .fold_real_expr(expr)
    }

    /// See [`constant_fold_real_expr`](crate::mir::Mir::constant_fold_real_expr)
    pub fn constant_fold_int_expr(
        &mut self,
        expr: IntegerExpressionId,
        resolver: &mut impl ConstResolver,
    ) -> Option<i64> {
        ConstantFold {
            fold_type: &mut MutatingConstantFold(self),
            resolver,
            expr,
        }
        .fold_integer_expr(expr)
    }

    /// See [`constant_fold_str_expr`](crate::mir::Mir::constant_fold_str_expr)
    pub fn constant_fold_str_expr(
        &mut self,
        expr: StringExpressionId,
        resolver: &mut impl ConstResolver,
    ) -> Option<StringLiteral> {
        ConstantFold {
            fold_type: &mut MutatingConstantFold(self),
            resolver,
            expr,
        }
        .fold_string_expr(expr)
    }

    /// Same as [`constant_fold_str_expr`](crate::mir::Mir::constant_fold_str_expr) but no expressions in `self` actually change only the return value is calculated
    pub fn constant_eval_real_expr(
        &self,
        expr: RealExpressionId,
        resolver: &mut impl ConstResolver,
    ) -> Option<f64> {
        ConstantFold {
            fold_type: &mut ReadingConstantFold(self),
            resolver,
            expr,
        }
        .fold_real_expr(expr)
    }

    /// See [`constant_eval_real_expr`](crate::mir::Mir::constant_eval_real_expr)
    pub fn constant_eval_int_expr(
        &self,
        expr: IntegerExpressionId,
        resolver: &mut impl ConstResolver,
    ) -> Option<i64> {
        ConstantFold {
            fold_type: &mut ReadingConstantFold(self),
            resolver,
            expr,
        }
        .fold_integer_expr(expr)
    }

    /// See [`constant_eval_real_expr`](crate::mir::Mir::constant_eval_real_expr)
    pub fn constant_eval_str_expr(
        &self,
        expr: StringExpressionId,
        resolver: &mut impl ConstResolver,
    ) -> Option<StringLiteral> {
        ConstantFold {
            fold_type: &mut ReadingConstantFold(self),
            resolver,
            expr,
        }
        .fold_string_expr(expr)
    }
}