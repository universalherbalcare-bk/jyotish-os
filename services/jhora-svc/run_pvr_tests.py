"""Run PyJHora's own regression suite (vendor/pyjhora/src/jhora/tests/pvr_tests.py) headless with LAHIRI.

pvr_tests.py's `if __name__ == "__main__"` block hard-codes TRUE_PUSHYA, prompts on stdin and stops
on the first failure. vendor/ is never modified, so this runner replays that block's configuration
steps (read from pvr_tests.py lines 9035-9110, PyJHora 5.0) with:
  * ayanamsa forced to LAHIRI
  * no interactive confirmation, no stop-on-fail (so the final "Total Tests ... #Failed Tests ..."
    line reflects the whole suite)
  * --nodes mean  -> the suite's own LAHIRI configuration (mean nodes + baseline
                     test_outputs_lahiri_mean_nodes.json in 'compare' mode)
  * --nodes true  -> the jhora-svc configuration (true nodes), baseline disabled, book values only

Usage: .venv/bin/python run_pvr_tests.py --nodes mean|true [--baseline compare|none]
Exit code: 0 when #Failed Tests == 0, 1 otherwise, 2 on harness error.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import os
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--nodes", choices=("mean", "true"), required=True)
    ap.add_argument("--baseline", choices=("compare", "none"), default=None)
    args = ap.parse_args()
    baseline_mode = args.baseline or ("compare" if args.nodes == "mean" else "none")

    from jhora_svc import (
        bootstrap,  # LAHIRI + no network + frozen settings, before drik import
    )

    bootstrap.bootstrap()
    from jhora import const, utils
    from jhora.panchanga import drik
    from jhora.tests import test_helper

    with contextlib.redirect_stdout(io.StringIO()):
        from jhora.tests import pvr_tests

    run_ayanamsa_mode = "LAHIRI"
    test_helper.set_stop_on_fail(False)
    use_true = args.nodes == "true"
    # exactly what pvr_tests.__main__ does for its LAHIRI branch, with the node choice parameterised
    const._use_true_nodes_for_rahu_ketu = use_true
    drik.set_planet_list(
        set_rahu_ketu_as_true_nodes=use_true, include_western_planets=False
    )
    # (the suite does not call const.set_node_mode(); replicated verbatim so config A == upstream)
    drik.set_ayanamsa_mode(run_ayanamsa_mode)
    lang = "en"
    const._DEFAULT_LANGUAGE = lang
    const.use_24hour_format_in_to_dms = False
    const.dhasa_year_duration_default = const.DHASA_YEAR_DURATION.MEAN_SIDEREAL_YEAR
    utils.set_language(lang)

    base_dir = Path(pvr_tests.__file__).resolve().parent
    baseline_file = base_dir / "test_outputs_lahiri_mean_nodes.json"
    test_helper.set_baseline(mode=baseline_mode, file=str(baseline_file))
    test_helper.set_baseline_write_mode("actual")
    if baseline_mode == "record":
        raise SystemExit("refusing to write into vendor/")

    summary = [
        "jhora-svc PyJHora pvr_tests runner",
        f"Baseline mode        : {baseline_mode}",
        f"Baseline file        : {baseline_file if baseline_mode == 'compare' else '-'}",
        f"Stop on fail         : {test_helper._STOP_IF_ANY_TEST_FAILED}",
        f"Ayanamsa mode        : {run_ayanamsa_mode} (const._DEFAULT_AYANAMSA_MODE={const._DEFAULT_AYANAMSA_MODE})",
        f"Rahu/Ketu true nodes : {const._use_true_nodes_for_rahu_ketu}",
        f"Dhasa year duration  : {const.dhasa_year_duration_default.name}",
        f"pid                  : {os.getpid()}",
    ]
    test_helper.show_configuration_summary(summary)

    start = time.time()
    exit_code = 0
    try:
        pvr_tests.all_unit_tests()
    except SystemExit as e:  # pragma: no cover - only if a test calls exit()
        exit_code = int(e.code) if e.code is not None else 0
    except Exception as exc:  # noqa: BLE001 - report harness crashes rather than hide them
        print(f"HARNESS ERROR: {type(exc).__name__}: {exc}")
        import traceback

        traceback.print_exc()
        exit_code = 2
    finally:
        sequence_total, executed_total, failed, failed_str, pass_pct, skipped_total = (
            test_helper.get_test_stats()
        )
        if executed_total == 0:
            print("No tests executed.")
        else:
            print(
                f"Total Tests {executed_total} #Failed Tests {failed}  Tests Passed (%) {pass_pct} %",
                failed_str,
            )
        print(
            f"sequence_total={sequence_total} skipped={skipped_total} elapsed_s={time.time() - start:.1f}"
        )
        print(
            f"CONFIG nodes={args.nodes} baseline={baseline_mode} ayanamsa={const._DEFAULT_AYANAMSA_MODE}"
        )
    if exit_code == 0 and failed:
        exit_code = 1
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
