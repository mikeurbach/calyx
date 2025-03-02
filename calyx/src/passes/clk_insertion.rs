use crate::ir::{
    self,
    traversal::{Named, Visitor},
    LibrarySignatures,
};
use ir::traversal::{Action, VisResult};

#[derive(Default)]
/// Adds assignments from a components `clk` port to every
/// component that contains an input `clk` port.
pub struct ClkInsertion;

impl Named for ClkInsertion {
    fn name() -> &'static str {
        "clk-insertion"
    }

    fn description() -> &'static str {
        "inserts assignments from component clk to sub-component clk"
    }
}

impl Visitor for ClkInsertion {
    fn start(
        &mut self,
        comp: &mut ir::Component,
        sigs: &LibrarySignatures,
        _comps: &[ir::Component],
    ) -> VisResult {
        let builder = ir::Builder::new(comp, sigs);

        for cell_ref in builder.component.cells.iter() {
            let cell = cell_ref.borrow();
            if let Some(port) = cell.find_with_attr("clk") {
                builder.component.continuous_assignments.push(
                    builder.build_assignment(
                        port,
                        builder
                            .component
                            .signature
                            .borrow()
                            .get_with_attr("clk"),
                        ir::Guard::True,
                    ),
                )
            }
        }

        // we don't need to traverse control
        Ok(Action::Stop)
    }
}
