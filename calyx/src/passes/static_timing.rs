use super::math_utilities::get_bit_width_from;
use crate::errors::CalyxResult;
use crate::ir::traversal::{Action, Named, VisResult, Visitor};
use crate::ir::{self, LibrarySignatures};
use crate::passes::RemoveCombGroups;
use crate::{build_assignments, errors::Error, guard, structure};
use itertools::Itertools;
use std::{cmp, rc::Rc};

#[derive(Default)]
/// Optimized lowering for control statements that only contain groups with
/// the "static" attribute.
///
/// Structured as a bottom-up pass. Control constructs not compiled by this
/// pass are compiled by the generic `CompileControl` pass.
pub struct StaticTiming {}

impl Named for StaticTiming {
    fn name() -> &'static str {
        "static-timing"
    }

    fn description() -> &'static str {
        "Opportunistically compile timed groups and generate timing information when possible."
    }
}

/// Function to iterate over a vector of control statements and collect
/// the "static" attribute using the `acc` function.
/// Returns None if any of of the Control statements is a compound statement.
fn accumulate_static_time<F>(stmts: &[ir::Control], acc: F) -> Option<u64>
where
    F: FnMut(u64, u64) -> u64,
{
    stmts
        .iter()
        .map(|con| {
            if let ir::Control::Enable(data) = con {
                data.group.borrow().attributes.get("static").copied()
            } else {
                None
            }
        })
        .fold_options(0, acc)
}

/// Attempts to get the value of the "static" attribute from the group if
/// present. Also ensures that the group is not a combinational group.
fn check_not_comb(group: &ir::RRC<ir::Group>) -> CalyxResult<Option<u64>> {
    if let Some(&time) = group.borrow().attributes.get("static") {
        if time < 1 {
            return Err(Error::MalformedControl(format!("static-timing: Group `{}` is a combinational group (it takes less than one cycle to run). Run `{}` to remove all combinational groups before running static-timing.", group.borrow().name(), RemoveCombGroups::name())));
        } else {
            Ok(Some(time))
        }
    } else {
        Ok(None)
    }
}

impl Visitor for StaticTiming {
    fn finish_while(
        &mut self,
        wh: &mut ir::While,
        comp: &mut ir::Component,
        ctx: &LibrarySignatures,
    ) -> VisResult {
        todo!()
        /* if let ir::Control::Enable(data) = &*wh.body {
            let cond = &wh.cond;
            let port = &wh.port;
            let body = &data.group;
            let mut builder = ir::Builder::new(comp, ctx);

            // The group is statically compilable
            //
            // Encoding: `b` latency of body, `c` is latency of cond
            //  cond[go] = fsm.out < c ? 1;
            //  cond_stored.in = fsm.out == c ? cond_port;
            //  cond_stored.write_en = fsm.out == c ? 1'd1;
            //  body[go] =
            //      fsm.out >= c & fsm.out < (b+c+1) & cond_stored.out ? 1'd1;
            //
            //  fsm.in = fsm.out <= c ? (fsm.out + 1);
            //  fsm.in =
            //      fsm.out >= (c+1) & fsm.out < b & cond_stored ? (fsm.out + 1);
            //  fsm.in = fsm.out == (b+c+1) ? 0;
            //  static_while[done] = fsm.out == (c+1) & !cond_stored ? 1;
            if let (Some(ctime), Some(btime)) =
                (check_not_comb(cond)?, check_not_comb(body)?)
            {
                let while_group = builder.add_group("static_while");

                let body_end_time = ctime + btime + 1;
                // `0` state + (ctime + btime) states.
                let fsm_size = get_bit_width_from(body_end_time + 1);

                structure!(builder;
                    let fsm = prim std_reg(fsm_size);
                    let cond_stored = prim std_reg(1);
                    let fsm_reset_val = constant(0, fsm_size);
                    let fsm_one = constant(1, fsm_size);
                    let incr = prim std_add(fsm_size);

                    let signal_on = constant(1, 1);

                    let cond_time_const = constant(ctime, fsm_size);
                    let cond_next = constant(ctime + 1, fsm_size);
                    let body_end_const = constant(body_end_time, fsm_size);
                );

                // Cond group will signal done on this cycle.
                let cond_done =
                    guard!(fsm["out"]).eq(guard!(cond_time_const["out"]));
                let body_done =
                    guard!(fsm["out"]).eq(guard!(body_end_const["out"]));

                // Should we increment the FSM this cycle.
                let fsm_cond_incr =
                    guard!(fsm["out"]).lt(guard!(cond_next["out"]));
                let fsm_body_incr = guard!(fsm["out"])
                    .ge(guard!(cond_next["out"]))
                    & guard!(fsm["out"]).lt(guard!(body_end_const["out"]))
                    & guard!(cond_stored["out"]);
                let fsm_incr = fsm_cond_incr | fsm_body_incr;

                // Compute the cond group
                let cond_go =
                    guard!(fsm["out"]).lt(guard!(cond_time_const["out"]));

                let body_go = guard!(cond_stored["out"])
                    & guard!(fsm["out"]).ge(guard!(cond_next["out"]))
                    & guard!(fsm["out"]).lt(guard!(body_end_const["out"]));

                let done = guard!(fsm["out"]).eq(guard!(cond_next["out"]))
                    & !guard!(cond_stored["out"]);

                let mut assignments = build_assignments!(
                    builder;
                    // Increment the FSM when needed
                    incr["left"] = ? fsm["out"];
                    incr["right"] = ? fsm_one["out"];
                    fsm["in"] = fsm_incr ? incr["out"];
                    fsm["write_en"] = fsm_incr ? signal_on["out"];

                    // Compute the cond group and save the result
                    cond["go"] = cond_go ? signal_on["out"];
                    cond_stored["write_en"] = cond_done ? signal_on["out"];

                    // Compute the body
                    body["go"] = body_go ? signal_on["out"];

                    // Reset the FSM when the body is done.
                    fsm["in"] = body_done ? fsm_reset_val["out"];
                    fsm["write_en"] = body_done ? signal_on["out"];

                    // This group is done when cond is false.
                    while_group["done"] = done ? signal_on["out"];
                );

                assignments.push(builder.build_assignment(
                    cond_stored.borrow().get("in"),
                    Rc::clone(port),
                    cond_done,
                ));

                while_group
                    .borrow_mut()
                    .assignments
                    .append(&mut assignments);

                // CLEANUP: Reset the FSM state.
                let mut cleanup = build_assignments!(
                    builder;
                    fsm["in"] = done ? fsm_reset_val["out"];
                    fsm["write_en"] = done ? signal_on["out"];
                );
                comp.continuous_assignments.append(&mut cleanup);

                return Ok(Action::Change(ir::Control::enable(while_group)));
            }
        }

        Ok(Action::Continue)
        */
    }

    fn finish_if(
        &mut self,
        s: &mut ir::If,
        comp: &mut ir::Component,
        ctx: &LibrarySignatures,
    ) -> VisResult {
        if let (ir::Control::Enable(tdata), ir::Control::Enable(fdata)) =
            (&*s.tbranch, &*s.fbranch)
        {
            let tru = &tdata.group;
            let fal = &fdata.group;

            if s.cond.is_some() {
                return Err(Error::MalformedStructure(format!("{}: condition group should be removed from if. Run `{}` before this pass.", Self::name(), RemoveCombGroups::name())));
            }

            if let (Some(ttime), Some(ftime)) =
                (check_not_comb(tru)?, check_not_comb(fal)?)
            {
                todo!()
                /* let mut builder = ir::Builder::new(comp, ctx);
                let if_group = builder.add_group("static_if");
                if_group
                    .borrow_mut()
                    .attributes
                    .insert("static", 1 + cmp::max(ttime, ftime));

                let end_true_time = ttime + 1;
                let end_false_time = ftime + 1;
                // `0` state + (ctime + max(ttime, ftime) + 1) states.
                let fsm_size = get_bit_width_from(
                    1 + cmp::max(end_true_time, end_false_time),
                );
                structure!(builder;
                    let fsm = prim std_reg(fsm_size);
                    let one = constant(1, fsm_size);
                    let signal_on = constant(1, 1);
                    let cond_stored = prim std_reg(1);
                    let reset_val = constant(0, fsm_size);

                    let true_end_const = constant(end_true_time, fsm_size);
                    let false_end_const = constant(end_false_time, fsm_size);

                    let incr = prim std_add(fsm_size);
                );

                let max_const = if ttime > ftime {
                    true_end_const.clone()
                } else {
                    false_end_const.clone()
                };

                // The group is done when we count up to the max.
                let done_guard =
                    guard!(fsm["out"]).eq(guard!(max_const["out"]));
                let not_done_guard = !done_guard.clone();

                // Guard for computing the conditional.
                let cond_go = if ctime == 0 {
                    guard!(fsm["out"]).eq(guard!(cond_time_const["out"]))
                } else {
                    guard!(fsm["out"]).lt(guard!(cond_time_const["out"]))
                };

                // Guard for when the conditional value is available on the
                // port.
                let cond_done =
                    guard!(fsm["out"]).eq(guard!(cond_done_time_const["out"]));

                // Guard for branches
                let true_go = guard!(fsm["out"])
                    .gt(guard!(cond_time_const["out"]))
                    & guard!(fsm["out"]).lt(guard!(true_end_const["out"]))
                    & guard!(cond_stored["out"]);

                let false_go = guard!(fsm["out"])
                    .gt(guard!(cond_time_const["out"]))
                    & guard!(fsm["out"]).lt(guard!(false_end_const["out"]))
                    & !guard!(cond_stored["out"]);

                let save_cond = builder.build_assignment(
                    cond_stored.borrow().get("in"),
                    Rc::clone(&s.port),
                    cond_done.clone(),
                );
                let mut assigns = build_assignments!(builder;
                    // Increment fsm every cycle till end
                    incr["left"] = ? fsm["out"];
                    incr["right"] = ? one["out"];
                    fsm["in"] = not_done_guard ? incr["out"];
                    fsm["write_en"] = not_done_guard ? signal_on["out"];

                    // Compute the cond group
                    cond["go"] = cond_go ? signal_on["out"];

                    // Store the value of the conditional
                    cond_stored["write_en"] = cond_done ? signal_on["out"];

                    // Enable one of the branches
                    tru["go"] = true_go ? signal_on["out"];
                    fal["go"] = false_go ? signal_on["out"];

                    // Group is done when we've counted up to max.
                    if_group["done"] = done_guard ? signal_on["out"];
                );
                if_group.borrow_mut().assignments.append(&mut assigns);
                if_group.borrow_mut().assignments.push(save_cond);

                // CLEANUP: Reset FSM to 0 when computation is finished.
                let mut clean_assigns = build_assignments!(builder;
                    fsm["in"] = done_guard ? reset_val["out"];
                    fsm["write_en"] = done_guard ? signal_on["out"];
                );
                comp.continuous_assignments.append(&mut clean_assigns);

                return Ok(Action::Change(ir::Control::enable(if_group)));
                */
            }
        }

        Ok(Action::Continue)
    }

    fn finish_par(
        &mut self,
        s: &mut ir::Par,
        comp: &mut ir::Component,
        ctx: &LibrarySignatures,
    ) -> VisResult {
        // Early return if this group is not compilable.
        if let Some(max_time) = accumulate_static_time(&s.stmts, cmp::max) {
            let mut builder = ir::Builder::new(comp, ctx);

            let par_group = builder.add_group("static_par");
            par_group.borrow_mut().attributes.insert("static", max_time);

            let fsm_size = get_bit_width_from(max_time + 1);

            structure!(builder;
                let fsm = prim std_reg(fsm_size);
                let signal_const = constant(1, 1);
                let incr = prim std_add(fsm_size);
                let one = constant(1, fsm_size);
                let last = constant(max_time, fsm_size);
            );
            let done_guard = guard!(fsm["out"]).eq(guard!(last["out"]));
            let not_done_guard = !done_guard.clone();

            let mut assigns = build_assignments!(builder;
                incr["left"] = ? one["out"];
                incr["right"] = ? fsm["out"];
                fsm["in"] = not_done_guard ? incr["out"];
                fsm["write_en"] = not_done_guard ? signal_const["out"];
                par_group["done"] = done_guard ? signal_const["out"];
            );
            par_group.borrow_mut().assignments.append(&mut assigns);
            for con in s.stmts.iter() {
                if let ir::Control::Enable(data) = con {
                    let group = &data.group;
                    let static_time: u64 =
                        *group.borrow().attributes.get("static").unwrap();

                    // group[go] = fsm.out <= static_time ? 1;
                    structure!(builder;
                        let state_const = constant(static_time, fsm_size);
                    );
                    let go_guard =
                        guard!(fsm["out"]).lt(guard!(state_const["out"]));

                    let mut assigns = build_assignments!(builder;
                      group["go"] = go_guard ? signal_const["out"];
                    );
                    par_group.borrow_mut().assignments.append(&mut assigns);
                }
            }

            // CLEANUP: Reset the FSM to initial state.
            structure!(builder;
                let reset_val = constant(0, fsm_size);
            );
            let mut cleanup_assigns = build_assignments!(builder;
                fsm["in"] = done_guard ? reset_val["out"];
                fsm["write_en"] = done_guard ? signal_const["out"];
            );
            comp.continuous_assignments.append(&mut cleanup_assigns);

            Ok(Action::Change(ir::Control::enable(par_group)))
        } else {
            Ok(Action::Continue)
        }
    }

    fn finish_seq(
        &mut self,
        s: &mut ir::Seq,
        comp: &mut ir::Component,
        ctx: &LibrarySignatures,
    ) -> VisResult {
        // If this sequence only contains groups with the "static" attribute,
        // compile it using a statically timed FSM.
        let total_time = accumulate_static_time(&s.stmts, |acc, x| acc + x);

        // Early return if this group is not compilable.
        if total_time.is_none() {
            return Ok(Action::Continue);
        }

        let mut builder = ir::Builder::new(comp, ctx);
        let fsm_size = get_bit_width_from(1 + total_time.unwrap());

        // Create new group for compiling this seq.
        let seq_group = builder.add_group("static_seq");

        // Add FSM register
        structure!(builder;
            let fsm = prim std_reg(fsm_size);
            let signal_const = constant(1, 1);
        );

        let mut cur_cycle = 0;
        for con in s.stmts.iter() {
            if let ir::Control::Enable(data) = con {
                let group = &data.group;

                // Static time of the group.
                let static_time: u64 =
                    *group.borrow().attributes.get("static").unwrap();

                structure!(builder;
                    let start_st = constant(cur_cycle, fsm_size);
                    let end_st = constant(cur_cycle + static_time, fsm_size);
                );

                // group[go] = fsm.out >= start_st & fsm.out < end_st ? 1;
                // NOTE(rachit): Do not generate fsm.out >= 0. Because fsm
                // contains unsigned values, it will always be true and
                // Verilator will generate %Warning-UNSIGNED.
                let go_guard = if static_time == 1 {
                    guard!(fsm["out"]).eq(guard!(start_st["out"]))
                } else if cur_cycle == 0 {
                    guard!(fsm["out"]).lt(guard!(end_st["out"]))
                } else {
                    guard!(fsm["out"]).ge(guard!(start_st["out"]))
                        & guard!(fsm["out"]).lt(guard!(end_st["out"]))
                };

                let mut assigns = build_assignments!(builder;
                    group["go"] = go_guard ? signal_const["out"];
                );
                seq_group.borrow_mut().assignments.append(&mut assigns);

                cur_cycle += static_time;
            }
        }

        // Add self incrementing logic for the FSM.
        structure!(builder;
            let incr = prim std_add(fsm_size);
            let one = constant(1, fsm_size);
            let last = constant(cur_cycle, fsm_size);
            let reset_val = constant(0, fsm_size);
        );
        let done_guard = guard!(fsm["out"]).eq(guard!(last["out"]));
        let not_done_guard = !done_guard.clone();

        let mut incr_assigns = build_assignments!(builder;
            incr["left"] = ? one["out"];
            incr["right"] = ? fsm["out"];
            fsm["in"] = not_done_guard ? incr["out"];
            fsm["write_en"] = not_done_guard ? signal_const["out"];
            seq_group["done"] = done_guard ? signal_const["out"];
        );
        seq_group.borrow_mut().assignments.append(&mut incr_assigns);

        // CLEANUP: Reset the fsm to initial state once it's done.
        let mut cleanup_assigns = build_assignments!(builder;
            fsm["in"] = done_guard ? reset_val["out"];
            fsm["write_en"] = done_guard ? signal_const["out"];
        );
        comp.continuous_assignments.append(&mut cleanup_assigns);

        // Add static attribute to this group.
        seq_group
            .borrow_mut()
            .attributes
            .insert("static", cur_cycle);

        // Replace the control with the seq group.
        Ok(Action::Change(ir::Control::enable(seq_group)))
    }
}
