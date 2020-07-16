//! This module contains an algorithm that will calculate all partial derivatives specified in a map for the entire programm
use crate::cfg::ControlFlowGraph;
use crate::derivatives::error::Error;
use crate::derivatives::{AutoDiff, Unknown};
use crate::diagnostic::MultiDiagnostic;
use crate::ir::{StatementId, VariableId};
use crate::mir::{ExpressionId, Mir, Statement};

impl ControlFlowGraph {
    pub fn demand_derivatives(
        &mut self,
        mir: &mut Mir,
        mut derivative_predicate: impl FnMut(&mut AutoDiff, VariableId, StatementId, Unknown) -> bool,
        upperbound: f64,
    ) -> MultiDiagnostic<Error> {
        // Try to approximate an upper bound by which a statement count is increased
        let factor = upperbound + 1.0;

        let mut ad = AutoDiff::new(mir);

        for (_, block) in self.postorder_itermut() {
            let upperbound_approx = (factor * block.statements.len() as f64).ceil() as usize;
            let mut old = Vec::with_capacity(upperbound_approx);
            std::mem::swap(&mut old, &mut block.statements);

            for stmt in old.into_iter().rev() {
                block.statements.push(stmt);
                ad.stmt_derivatives(&mut block.statements, stmt, &mut derivative_predicate)
            }

            block.statements.reverse();
        }

        self.statement_owner_cache
            .invalidate(ad.mir.statements.len());
        ad.errors
    }

    /// Use this with care calling this on multiple overlapping cfgs will calculate derivatives multiple times
    pub fn calculate_all_registered_derivatives(
        &mut self,
        mir: &mut Mir,
    ) -> MultiDiagnostic<Error> {
        let derivative_count = mir.derivatives.values().flatten().count();

        // * 2 to take transitive dependencies into account
        let upperbound = (2 * derivative_count) as f64 / mir.variables.len() as f64;
        self.demand_derivatives(mir, |_, _, _, _| true, upperbound)
    }
}

impl<'lt> AutoDiff<'lt> {
    pub fn stmt_derivatives(
        &mut self,
        dst: &mut Vec<StatementId>,
        stmt: StatementId,
        derivative_predicate: &mut impl FnMut(&mut AutoDiff, VariableId, StatementId, Unknown) -> bool,
    ) {
        if let Statement::Assignment(attr, var, expr) = self.mir[stmt] {
            if let Some(partial_derivatives) = self.mir.derivatives.get(&var).cloned() {
                for (derive_by, derivative_var) in partial_derivatives {
                    if !derivative_predicate(self, var, stmt, derive_by) {
                        continue;
                    }

                    let derivative = self.partial_derivative(expr, derive_by);

                    let stmt = self.mir.statements.push(Statement::Assignment(
                        attr,
                        derivative_var,
                        ExpressionId::Real(derivative),
                    ));

                    dst.push(stmt);
                    // Higher order derivatives
                    self.stmt_derivatives(dst, stmt, derivative_predicate)
                }
            }
        }
    }
}