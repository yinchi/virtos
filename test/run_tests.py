#!/usr/bin/env python3
"""Boots virtos headless and runs every test case module in `cases/` against it.

Usage: run_tests.py --kernel virtos.elf --system system.img --mkimage <path> [MODULE...]
(normally via `just test-qemu`; module names select which to run).

Each module runs in **its own group**: its own QEMU instance, its own private copy of the system
image and, unless it says otherwise, its own fresh HOME disk. Groups run in a thread pool, at most
`cpus - 1` at a time (`QEMU_TEST_WORKERS=N` overrides), and their results are printed afterwards in
a fixed order, so the transcript reads the same however the sessions interleaved. After QEMU has
quit, each group runs its module's `verify_disk` and `fsck.fat -n` on every image it used.

A module exposes `run(ctx)`, and optionally:

- `verify_disk(ctx)`: checks of the images once QEMU is gone (`ctx.system_img`, `ctx.home_img`);
- `SYSTEM_FILES` / `HOME_FILES`: `{path: content}` put on the group's copy of the system image or
  on its fresh HOME disk before boot (`None` as content removes a file of the system image).
  Defaults: `harness.DEFAULT_ENVIRONMENT` as `/etc/environment`, an empty HOME disk;
- `TEST_PROGS = ["badcalls", ...]`: programs of `test/progs` (bad system calls, deliberate faults)
  put on the group's copy of the system image as `/tests/<name>` -- not executable: the module
  `chmod +x`es what it runs;
- `HOME = False`: boot with no HOME disk attached; `HOME_LABEL` / `SYSTEM_LABEL`: the volume labels
  (default HOME, and the system image's own SYSTEM);
- `EXTRA_DISKS = [{"label": ..., "id": "8 hex digits", "files": {...}}, ...]`: more volumes, attached
  after the HOME disk;
- `BOOT_FAILS = True`: the kernel is expected not to reach a prompt -- `run` reads the serial log
  with `Session.wait_for_log` instead;
- `EXCLUSIVE = True`: the group's checks are sensitive to CPU load (they race real key presses
  against the guest), so it runs alone, after every parallel group has finished;
- `RETRIES = n`: if the group fails, run it again, up to n more times, and report which attempt
  passed -- a flake is never silent. Both are for the few known-flaky groups: everything else runs
  once, at full speed.
"""

import argparse
import concurrent.futures
import importlib
import os
import pkgutil
import shutil
import subprocess
import sys
import tempfile
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import cases  # noqa: E402
import harness  # noqa: E402


def worker_count():
    override = os.environ.get("QEMU_TEST_WORKERS")
    if override:
        return max(1, int(override))
    try:
        cpus = len(os.sched_getaffinity(0))
    except AttributeError:
        cpus = os.cpu_count() or 4
    return max(1, cpus - 1)


def load_modules(names):
    found = sorted(m.name for m in pkgutil.iter_modules(cases.__path__))
    wanted = names or found
    unknown = [n for n in wanted if n not in found]
    if unknown:
        sys.exit(f"unknown test module(s): {', '.join(unknown)} (have: {', '.join(found)})")
    return [importlib.import_module(f"cases.{n}") for n in wanted]


def run_once(module, args):
    """One attempt at one module: returns its list of `(name, got, want)` check triples."""
    label = module.__name__.rsplit(".", 1)[-1]
    results = []

    def check(name, got, want):
        results.append((f"{label}: {name}", got, want))

    workdir = tempfile.mkdtemp(prefix=f"virtos-{label}-")
    system_img = os.path.join(workdir, "system.img")
    subprocess.run(["cp", "--sparse=always", args.system, system_img], check=True)
    system_files = {"etc/environment": harness.DEFAULT_ENVIRONMENT}
    system_files.update(getattr(module, "SYSTEM_FILES", {}))
    for path, content in system_files.items():
        harness.put_file(system_img, path, content)
    for name in getattr(module, "TEST_PROGS", []):
        with open(os.path.join(args.test_progs, name), "rb") as f:
            harness.put_file(system_img, f"tests/{name}", f.read())
    if hasattr(module, "SYSTEM_LABEL"):
        harness.relabel(system_img, module.SYSTEM_LABEL)
    home_img = None
    if getattr(module, "HOME", True):
        home_img = os.path.join(workdir, "home.img")
        harness.make_home_image(args.mkimage, home_img, workdir, getattr(module, "HOME_FILES", {}),
                                getattr(module, "HOME_LABEL", "HOME"))

    extra_imgs = []
    for n, spec in enumerate(getattr(module, "EXTRA_DISKS", [])):
        path = os.path.join(workdir, f"extra{n}.img")
        harness.make_home_image(args.mkimage, path, os.path.join(workdir, f"extra{n}"), spec.get("files", {}),
                                spec.get("label", "EXTRA"), spec.get("id", "00001111"))
        extra_imgs.append(path)

    session = None
    try:
        session = harness.Session(os.path.abspath(args.kernel), system_img, home_img, workdir,
                                  expect_boot=not getattr(module, "BOOT_FAILS", False), extra_imgs=extra_imgs)
        ctx = harness.Context(session, check, workdir, system_img, home_img, extra_imgs)
        module.run(ctx)
    except Exception:
        check("the module ran to completion", traceback.format_exc(), None)
    finally:
        if session:
            session.close()
    if session:
        ctx = harness.Context(session, check, workdir, system_img, home_img, extra_imgs)
        try:
            if hasattr(module, "verify_disk"):
                module.verify_disk(ctx)
            for name, img in (("system", system_img), ("home", home_img), *[(f"extra{i}", p) for i, p in enumerate(extra_imgs)]):
                if img:
                    fsck = subprocess.run(["fsck.fat", "-n", img], capture_output=True, text=True)
                    check(f"fsck.fat -n is clean on the {name} image",
                          0 if fsck.returncode == 0 else (fsck.returncode, fsck.stdout + fsck.stderr), 0)
        except Exception:
            check("verify_disk ran to completion", traceback.format_exc(), None)
    shutil.rmtree(workdir, ignore_errors=True)
    return results


def run_group(module, args):
    """Runs `module`, again after a failure if it says `RETRIES`: `(results, attempt)`."""
    attempts = 1 + getattr(module, "RETRIES", 0)
    for attempt in range(1, attempts + 1):
        results = run_once(module, args)
        if all(got == want for _, got, want in results):
            return results, attempt
    return results, attempts


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--kernel", required=True)
    parser.add_argument("--system", required=True)
    parser.add_argument("--mkimage", required=True)
    parser.add_argument("--test-progs", help="directory holding the built test/progs binaries")
    parser.add_argument("modules", nargs="*")
    args = parser.parse_args()

    modules = load_modules(args.modules)
    exclusive = [m for m in modules if getattr(m, "EXCLUSIVE", False)]
    shared = [m for m in modules if m not in exclusive]
    workers = max(1, min(len(shared), worker_count()))
    print(f"running {len(shared)} groups, {workers} at a time, then {len(exclusive)} on their own", flush=True)

    outcome = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        futures = [(m, pool.submit(run_group, m, args)) for m in shared]
        for m, f in futures:
            outcome[m] = f.result()
    for m in exclusive:  # nothing else is running now
        outcome[m] = run_group(m, args)

    failures = []
    for m in modules:
        results, attempt = outcome[m]
        if attempt > 1 and all(got == want for _, got, want in results):
            print(f"NOTE  {m.__name__.rsplit('.', 1)[-1]} passed only on attempt {attempt} (a flake)")
        for name, got, want in results:
            if got == want:
                print(f"PASS  {name}")
            else:
                print(f"FAIL  {name}\n  want: {want!r}\n  got:  {got!r}")
                failures.append(name)
    if failures:
        print(f"\n{len(failures)} failed")
        return 1
    print(f"\nall {sum(len(r) for r, _ in outcome.values())} checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
