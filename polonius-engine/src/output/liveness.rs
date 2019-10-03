// Copyright 2019 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! An implementation of the origin liveness calculation logic

use std::collections::BTreeSet;
use std::time::Instant;

use crate::output::Output;
use facts::FactTypes;

use datafrog::{Iteration, Relation, RelationLeaper};

pub(super) fn compute_live_regions<T: FactTypes>(
    var_used: Vec<(T::Variable, T::Point)>,
    var_drop_used: Vec<(T::Variable, T::Point)>,
    var_defined: Vec<(T::Variable, T::Point)>,
    var_uses_region: Vec<(T::Variable, T::Origin)>,
    var_drops_region: Vec<(T::Variable, T::Origin)>,
    cfg_edge: &[(T::Point, T::Point)],
    var_maybe_initialized_on_exit: Vec<(T::Variable, T::Point)>,
    output: &mut Output<T>,
) -> Vec<(T::Origin, T::Point)> {
    debug!("compute_liveness()");
    let computation_start = Instant::now();
    let mut iteration = Iteration::new();

    // Relations
    let var_defined_rel: Relation<(T::Variable, T::Point)> = var_defined.into();
    let cfg_edge_rel: Relation<(T::Point, T::Point)> =
        cfg_edge.iter().map(|(p, q)| (*p, *q)).collect();
    let cfg_edge_reverse_rel: Relation<(T::Point, T::Point)> =
        cfg_edge.iter().map(|(p, q)| (*q, *p)).collect();
    let var_uses_region_rel: Relation<(T::Variable, T::Origin)> = var_uses_region.into();
    let var_drops_region_rel: Relation<(T::Variable, T::Origin)> = var_drops_region.into();
    let var_maybe_initialized_on_exit_rel: Relation<(T::Variable, T::Point)> =
        var_maybe_initialized_on_exit.into();
    let var_drop_used_rel: Relation<((T::Variable, T::Point), ())> = var_drop_used
        .into_iter()
        .map(|(v, p)| ((v, p), ()))
        .collect();

    // T::Variables

    // `var_live`: variable V is live upon entry in point P
    let var_live_var = iteration.variable::<(T::Variable, T::Point)>("var_live_at");
    // `var_drop_live`: variable V is drop-live (will be used for a drop) upon entry in point P
    let var_drop_live_var = iteration.variable::<(T::Variable, T::Point)>("var_drop_live_at");

    // This is what we are actually calculating:
    let region_live_at_var = iteration.variable::<((T::Origin, T::Point), ())>("region_live_at");

    // This propagates the relation `var_live(V, P) :- var_used(V, P)`:
    var_live_var.insert(var_used.into());

    // var_maybe_initialized_on_entry(V, Q) :-
    //     var_maybe_initialized_on_exit(V, P),
    //     cfg_edge(P, Q).
    let var_maybe_initialized_on_entry = Relation::from_leapjoin(
        &var_maybe_initialized_on_exit_rel,
        cfg_edge_rel.extend_with(|&(_v, p)| p),
        |&(v, _p), &q| ((v, q), ()),
    );

    // var_drop_live(V, P) :-
    //     var_drop_used(V, P),
    //     var_maybe_initialzed_on_entry(V, P).
    var_drop_live_var.insert(Relation::from_join(
        &var_drop_used_rel,
        &var_maybe_initialized_on_entry,
        |&(v, p), &(), &()| (v, p),
    ));

    while iteration.changed() {
        // region_live_at(R, P) :-
        //   var_drop_live(V, P),
        //   var_drops_region(V, R).
        region_live_at_var.from_join(&var_drop_live_var, &var_drops_region_rel, |_v, &p, &r| {
            ((r, p), ())
        });

        // region_live_at(R, P) :-
        //   var_live(V, P),
        //   var_uses_region(V, R).
        region_live_at_var.from_join(&var_live_var, &var_uses_region_rel, |_v, &p, &r| {
            ((r, p), ())
        });

        // var_live(V, P) :-
        //     var_live(V, Q),
        //     cfg_edge(P, Q),
        //     !var_defined(V, P).
        var_live_var.from_leapjoin(
            &var_live_var,
            (
                var_defined_rel.extend_anti(|&(v, _q)| v),
                cfg_edge_reverse_rel.extend_with(|&(_v, q)| q),
            ),
            |&(v, _q), &p| (v, p),
        );

        // var_drop_live(V, P) :-
        //     var_drop_live(V, Q),
        //     cfg_edge(P, Q),
        //     !var_defined(V, P)
        //     var_maybe_initialized_on_exit(V, P).
        // extend p with v:s from q such that v is not in q, there is an edge from p to q
        var_drop_live_var.from_leapjoin(
            &var_drop_live_var,
            (
                var_defined_rel.extend_anti(|&(v, _q)| v),
                cfg_edge_reverse_rel.extend_with(|&(_v, q)| q),
                var_maybe_initialized_on_exit_rel.extend_with(|&(v, _q)| v),
            ),
            |&(v, _q), &p| (v, p),
        );
    }

    let region_live_at_rel = region_live_at_var.complete();

    info!(
        "compute_liveness() completed: {} tuples, {:?}",
        region_live_at_rel.len(),
        computation_start.elapsed()
    );

    if output.dump_enabled {
        let var_drop_live_at = var_drop_live_var.complete();
        for &(var, location) in &var_drop_live_at.elements {
            output
                .var_drop_live_at
                .entry(location)
                .or_insert_with(Vec::new)
                .push(var);
        }

        let var_live_at = var_live_var.complete();
        for &(var, location) in &var_live_at.elements {
            output
                .var_live_at
                .entry(location)
                .or_insert_with(Vec::new)
                .push(var);
        }
    }

    region_live_at_rel
        .iter()
        .map(|&((r, p), _)| (r, p))
        .collect()
}

pub(super) fn make_universal_region_live<T: FactTypes>(
    region_live_at: &mut Vec<(T::Origin, T::Point)>,
    cfg_edge: &[(T::Point, T::Point)],
    universal_region: Vec<T::Origin>,
) {
    debug!("make_universal_regions_live()");

    let all_points: BTreeSet<T::Point> = cfg_edge
        .iter()
        .map(|&(p, _)| p)
        .chain(cfg_edge.iter().map(|&(_, q)| q))
        .collect();

    region_live_at.reserve(universal_region.len() * all_points.len());
    for &r in &universal_region {
        for &p in &all_points {
            region_live_at.push((r, p));
        }
    }
}

pub(super) fn init_region_live_at<T: FactTypes>(
    var_used: Vec<(T::Variable, T::Point)>,
    var_drop_used: Vec<(T::Variable, T::Point)>,
    var_defined: Vec<(T::Variable, T::Point)>,
    var_uses_region: Vec<(T::Variable, T::Origin)>,
    var_drops_region: Vec<(T::Variable, T::Origin)>,
    var_maybe_initialized_on_exit: Vec<(T::Variable, T::Point)>,
    cfg_edge: &[(T::Point, T::Point)],
    universal_region: Vec<T::Origin>,
    output: &mut Output<T>,
) -> Vec<(T::Origin, T::Point)> {
    debug!("init_region_live_at()");
    let mut region_live_at = compute_live_regions(
        var_used,
        var_drop_used,
        var_defined,
        var_uses_region,
        var_drops_region,
        cfg_edge,
        var_maybe_initialized_on_exit,
        output,
    );

    make_universal_region_live::<T>(&mut region_live_at, cfg_edge, universal_region);

    region_live_at
}
