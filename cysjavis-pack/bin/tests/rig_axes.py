#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""rig_axes.py — the axes a test fixture must seal, named one by one (CEO A9).

A fixture that seals HOME and leaves cwd in a worktree can ascend to a real ~/_round, read it and pass, or write to it.
That happened here: one row reached the live base ledger. So each axis is asserted by name before any operation.
"""
import os

AXES = ("cwd", "HOME", "USERPROFILE", "LOCALAPPDATA", "CYS_SOCKET", "CYS_STATE_DIR", "CYS_PACK_DIR", "JAVIS_ROOT",
        "CYS_PROBE_RUNS")
# These lanes have none by design. Unset then falls to the HOME axis, which is asserted.
MAY_BE_UNSET = ("CYS_SOCKET",)


def inside(path, rig):
    real, top = os.path.realpath(path), os.path.realpath(rig)
    return real == top or real.startswith(top + os.sep)


def breaches(env, cwd, rig):
    """{axis: why} for every axis that is not inside the rig."""
    out = {}
    for axis in AXES:
        value = cwd if axis == "cwd" else env.get(axis)
        if not value:
            if axis not in MAY_BE_UNSET:
                out[axis] = "unset"
        elif not inside(value, rig):
            out[axis] = value
    return out


def assert_sealed(case, env, rig):
    """One assertion per axis, so a breach names itself. Runs before the operation it guards."""
    bad = breaches(env, os.getcwd(), rig)
    for axis in AXES:
        with case.subTest(axis=axis):
            case.assertNotIn(axis, bad)
