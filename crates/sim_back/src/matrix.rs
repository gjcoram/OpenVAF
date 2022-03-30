use ahash::AHashMap;
use bitset::BitSet;
use hir_def::db::HirDefDB;
use hir_def::NodeId;
use hir_lower::{CallBackKind, HirInterner, ParamKind, PlaceKind};
use hir_ty::db::HirTyDB;
use hir_ty::lower::BranchKind;
use indexmap::map::Entry;
use mir::builder::InstBuilder;
use mir::cursor::FuncCursor;
use mir::{Function, Value, F_ZERO};
use mir_autodiff::FirstOrderUnkown;
use stdx::{format_to, impl_debug_display, impl_idx_from};
use typed_indexmap::TiMap;

use crate::compilation_db::CompilationDB;

#[derive(PartialEq, Eq, Clone, Copy, Hash)]
pub struct MatrixEntry {
    pub row: NodeId,
    pub col: NodeId,
}
impl_debug_display! {match MatrixEntry{MatrixEntry{row, col} => "({row:?}, {col:?})";}}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct MatrixEntryId(u32);
impl_idx_from!(MatrixEntryId(u32));
impl_debug_display! {match MatrixEntryId{MatrixEntryId(id) => "j{id}";}}

#[derive(Default, Debug)]
pub struct JacobianMatrix {
    pub resistive: TiMap<MatrixEntryId, MatrixEntry, Value>,
    pub reactive: TiMap<MatrixEntryId, MatrixEntry, Value>,
}

impl JacobianMatrix {
    pub(crate) fn populate(
        &mut self,
        db: &CompilationDB,
        func: &mut FuncCursor,
        intern: &HirInterner,
        derivatives: &AHashMap<(Value, FirstOrderUnkown), Value>,
    ) {
        for (out_kind, rhs) in intern.outputs.iter() {
            let (row_hi, row_lo, reactive) = match *out_kind {
                PlaceKind::BranchVoltage { .. } | PlaceKind::ImplicitBranchVoltage { .. } => {
                    todo!("voltage contribute")
                }
                PlaceKind::BranchCurrent { branch, reactive } => {
                    match db.branch_info(branch).unwrap().kind {
                        BranchKind::NodeGnd(node) => (node, None, reactive),
                        BranchKind::Nodes(hi, lo) => (hi, Some(lo), reactive),
                        BranchKind::PortFlow(_) => unreachable!(),
                    }
                }
                PlaceKind::ImplicitBranchCurrent { hi, lo, reactive } => (hi, lo, reactive),
                _ => continue,
            };

            let hi_gnd = db.node_data(row_hi).is_gnd;
            let lo_gnd = row_lo.map_or(true, |lo| db.node_data(lo).is_gnd);

            for (param, (kind, _)) in intern.params.iter_enumerated() {
                let (col_hi, col_lo) = match kind {
                    ParamKind::Voltage { hi, lo } => (*hi, *lo),
                    // ParamKind::Current(_) => TODO voltage contribute
                    _ => continue,
                };
                let unkown =
                    u32::from(intern.callbacks.unwrap_index(&CallBackKind::Derivative(param)));
                let unkown = FirstOrderUnkown::from(unkown);
                if let Some(ddx) = derivatives.get(&(*rhs, unkown)).copied() {
                    if ddx == F_ZERO {
                        continue;
                    }

                    if !hi_gnd {
                        self.ensure_entry(func, row_hi, col_hi, ddx, false, reactive);
                        if let Some(col_lo) = col_lo {
                            self.ensure_entry(func, row_hi, col_lo, ddx, true, reactive);
                        }
                    }

                    if let Some(row_lo) = row_lo {
                        if !lo_gnd {
                            self.ensure_entry(func, row_lo, col_hi, ddx, true, reactive);
                            if let Some(col_lo) = col_lo {
                                self.ensure_entry(func, row_lo, col_lo, ddx, false, reactive);
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn insert_opt_barries(
        &mut self,
        func: &mut FuncCursor,
        output_values: &mut BitSet<Value>,
    ) {
        fn insert_opt_barries(
            entries: &mut TiMap<MatrixEntryId, MatrixEntry, Value>,
            func: &mut FuncCursor,
            output_values: &mut BitSet<Value>,
        ) {
            for val in entries.raw.values_mut() {
                *val = func.ins().optbarrier(*val);
            }

            output_values.ensure(func.func.dfg.num_values() + 1);

            for val in entries.raw.values() {
                output_values.insert(*val);
            }
        }

        insert_opt_barries(&mut self.resistive, func, output_values);
        insert_opt_barries(&mut self.reactive, func, output_values);
    }

    pub(crate) fn strip_opt_barries(
        &mut self,
        func: &mut Function,
        output_values: &mut BitSet<Value>,
    ) {
        fn strip_opt_barries(
            entries: &mut TiMap<MatrixEntryId, MatrixEntry, Value>,
            func: &mut Function,
            output_values: &mut BitSet<Value>,
        ) {
            for val in entries.raw.values_mut() {
                let inst = func.dfg.value_def(*val).unwrap_inst();
                output_values.remove(*val);
                *val = func.dfg.instr_args(inst)[0];
                output_values.insert(*val);
                func.layout.remove_inst(inst);
            }
        }

        strip_opt_barries(&mut self.resistive, func, output_values);
        strip_opt_barries(&mut self.reactive, func, output_values);
    }

    pub(crate) fn ensure_entry(
        &mut self,
        func: &mut FuncCursor,
        row: NodeId,
        col: NodeId,
        mut val: Value,
        neg: bool,
        reactive: bool,
    ) {
        let dst = if reactive { &mut self.reactive } else { &mut self.resistive };
        // no entrys for gnd nodes

        match dst.raw.entry(MatrixEntry { row, col }) {
            Entry::Occupied(dst) => {
                let dst = dst.into_mut();
                *dst = if neg { func.ins().fsub(*dst, val) } else { func.ins().fadd(*dst, val) }
            }
            Entry::Vacant(dst) => {
                if neg {
                    val = func.ins().fneg(val);
                }
                dst.insert(val);
            }
        }
    }

    pub(crate) fn sparsify(&mut self) {
        self.resistive.raw.retain(|_, val| *val != F_ZERO);
        self.reactive.raw.retain(|_, val| *val != F_ZERO);
    }
}

impl JacobianMatrix {
    pub fn print_resistive_stamps(&self, db: &dyn HirDefDB) -> String {
        let mut res = String::new();
        for (entry, val) in &self.resistive.raw {
            format_to!(
                res,
                "({}, {}) = {}\n",
                db.node_data(entry.row).name,
                db.node_data(entry.col).name,
                val
            )
        }

        res
    }

    pub fn print_reactive_stamps(&self, db: &dyn HirDefDB) -> String {
        let mut res = String::new();
        for (entry, val) in &self.reactive.raw {
            format_to!(
                res,
                "({}, {}) = {}\n",
                db.node_data(entry.row).name,
                db.node_data(entry.col).name,
                val
            )
        }

        res
    }
}