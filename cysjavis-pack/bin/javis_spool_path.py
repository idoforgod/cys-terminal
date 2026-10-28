#!/usr/bin/env python3
"""javis_spool_path.py - the one source of truth for the HUD event spool path (0.14.56 X-8 CARRIER, invariant A).

The spool path is a function of the pack directory ONLY: <pack_dir>/state/evt_spool.jsonl.
Producers (javis_event._spool_path) and the reader (javis_hud_bridge, one spool per lane) both call
spool_path_for_pack(). No environment variable takes part (HUD_STATE_DIR is the bridge's private state dir,
not the spool's). The Rust daemon's own copy of this formula (cysd spool_watch W4) is pinned to this file's text.

Charter: stdlib only, imports no pack module, reads no environment, no side effect at import.
Callers load this file by explicit path (importlib.util.spec_from_file_location), never by a bare import:
under `python -I` the script directory is not on sys.path.
"""
import os

SPOOL_STATE_DIRNAME = "state"
SPOOL_BASENAME = "evt_spool.jsonl"


def spool_path_for_pack(pack_dir):
    """Return <pack_dir>/state/evt_spool.jsonl for a pack directory (os.path only, no normalisation)."""
    return os.path.join(pack_dir, SPOOL_STATE_DIRNAME, SPOOL_BASENAME)
