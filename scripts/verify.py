"""One command per ticket: run the checks, start a backend, run the contract tests, stop it.

    python scripts/verify.py checks                 tokens + contract typecheck
    python scripts/verify.py a [vitest filters...]  backend A: clippy, cargo test, D1 migrate, wrangler dev, contract
    python scripts/verify.py b [vitest filters...]  backend B: vet, go test, Postgres (Docker), migrate, server, contract
    python scripts/verify.py web                    frontend: types, Biome, tsc, Vitest, build, ds-check
    python scripts/verify.py e2e-a|e2e-b [filters]  Playwright + axe through the web app served by backend A or B

Filters go to vitest as-is, e.g. `test/auth.test.ts` or `-t misc`. Without filters the whole contract suite runs.
Conventions the backends must follow (docs/test-plan.md):
  A  backend-a/wrangler.toml has a D1 binding named DB, [vars] APP_ORIGIN, and serves on --port 8787
  B  `go run ./cmd/migrate up` and `go run ./cmd/server` read DATABASE_URL, APP_ORIGIN and PORT
Exit code 0 only when every step passed.
"""

import os
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WRANGLER = ["npx", "--yes", "wrangler@4.163.0"]
PG_IMAGE = "postgres:18.1-alpine"
PG_CONTAINER = "pilot-shortlink-verify-pg"
# rustup and the Go installer do not always put cargo and go on the Windows PATH; wrangler's build needs cargo too
os.environ["PATH"] = os.pathsep.join(
    [str(Path.home() / ".cargo" / "bin"), r"C:\Program Files\Go\bin", os.environ.get("PATH", "")]
)


def exe(cmd: list[str]) -> list[str]:
    path = shutil.which(cmd[0])  # resolves npx.cmd / pnpm.cmd on Windows
    if not path:
        sys.exit(f"verify: {cmd[0]} not found on PATH")
    return [path, *cmd[1:]]


def run(step: str, cmd: list[str], cwd: Path = ROOT, env: dict | None = None) -> None:
    print(f"\n== {step}: {' '.join(cmd)}", flush=True)
    if not cwd.is_dir():
        sys.exit(f"verify: FAILED at {step}: {cwd.relative_to(ROOT)}/ does not exist yet")
    if subprocess.run(exe(cmd), cwd=cwd, env=env).returncode != 0:
        sys.exit(f"verify: FAILED at {step}")


def start(cmd: list[str], cwd: Path, env: dict | None = None) -> subprocess.Popen:
    print(f"\n== start: {' '.join(cmd)}", flush=True)
    return subprocess.Popen(exe(cmd), cwd=cwd, env=env)


def stop(proc: subprocess.Popen) -> None:
    if proc.poll() is not None:
        return
    if os.name == "nt":  # wrangler starts workerd as a child; kill the whole tree
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True)
    else:
        proc.terminate()
    proc.wait(timeout=30)


def wait_healthy(url: str, proc: subprocess.Popen, timeout: int = 180) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if proc.poll() is not None:
            sys.exit(f"verify: server exited with {proc.returncode} before {url} was healthy")
        try:
            with urllib.request.urlopen(f"{url}/health", timeout=5) as r:
                if r.status == 200:
                    return
        except OSError:
            pass
        time.sleep(2)
    sys.exit(f"verify: {url}/health not ready after {timeout}s")


def contract(api_url: str, filters: list[str]) -> None:
    run("contract", ["pnpm", "-F", "contract", "exec", "vitest", "run", *filters], env={**os.environ, "API_URL": api_url})


def e2e(url: str, filters: list[str]) -> None:
    run("e2e", ["pnpm", "-F", "e2e", "test", *filters], env={**os.environ, "E2E_URL": url})


def checks() -> None:
    run("tokens", ["node", "design/build-tokens.mjs", "--check"])
    run("typecheck", ["pnpm", "-F", "contract", "typecheck"])


DS_CHECK = ROOT.parent / "TAWAN-OS-AGENT" / ".agents" / "skills" / "uxui-workflow" / "scripts" / "ds-check.mjs"


def web(_: list[str]) -> None:
    run("types in sync", ["pnpm", "-F", "web", "types"])
    run("biome", ["pnpm", "-F", "web", "lint"])
    run("typecheck", ["pnpm", "-F", "web", "typecheck"])
    run("unit", ["pnpm", "-F", "web", "test"])
    run("build", ["pnpm", "-F", "web", "build"])
    if DS_CHECK.exists():  # the checker lives in TAWAN-OS; CI does not have it
        run("ds-check", ["node", str(DS_CHECK), "check", "web/src", "--tokens", "design/tokens.json"])


FAKE_IDP = "http://127.0.0.1:9411"
# OAuth and Turnstile fakes (docs/test-plan.md section 9): CLI/env only, never in wrangler.toml or render.yaml,
# and the backends refuse them unless APP_ORIGIN is localhost (AC-O11)
FAKES = {"OAUTH_MODE": "fake", "OAUTH_FAKE_BASE": FAKE_IDP, "TURNSTILE_MODE": "fake"}


def start_fake_idp() -> subprocess.Popen:
    proc = start(["node", "contract/fake-idp.mjs", "9411"], cwd=ROOT)
    wait_healthy(FAKE_IDP, proc, timeout=30)
    return proc


def build_web() -> None:
    """Both backends serve web/dist (C8, ADR-001), so the app is built before a backend starts."""
    run("web build", ["pnpm", "-F", "web", "build"])


def backend_a(filters: list[str], tests=None) -> None:
    a = ROOT / "backend-a"
    build_web()
    run("clippy", ["cargo", "clippy", "--all-targets", "--", "-D", "warnings"], cwd=a)
    run("cargo test", ["cargo", "test"], cwd=a)
    run("migrate", [*WRANGLER, "d1", "migrations", "apply", "DB", "--local"], cwd=a)
    # Local D1 keeps state between runs; the contract suite alone makes ~21 OAuth callbacks against a 30 per 10 min
    # per-IP limit, so a second run inside 10 minutes would trip it. Start every run with empty counters.
    run("reset rate limits", [*WRANGLER, "d1", "execute", "DB", "--local", "--command", "DELETE FROM rate_limits"], cwd=a)
    url = "http://localhost:8787"
    # MAIL_MODE=capture only on the command line, never in wrangler.toml, so it cannot reach production (AC-V8)
    fake_vars = [arg for k, v in FAKES.items() for arg in ("--var", f"{k}:{v}")]
    idp = start_fake_idp()
    server = start([*WRANGLER, "dev", "--port", "8787", "--var", "MAIL_MODE:capture", *fake_vars], cwd=a)
    try:
        wait_healthy(url, server)
        (tests or contract)(url, filters)
    finally:
        stop(server)
        stop(idp)


def backend_b(filters: list[str], tests=None) -> None:
    b = ROOT / "backend-b"
    build_web()
    # Go's embed cannot reach outside the module, so the built app is copied in (backend-b/internal/site)
    site = b / "internal" / "site" / "dist"
    if site.is_dir():
        for old in site.iterdir():  # keep the committed .keep placeholder
            if old.name != ".keep":
                shutil.rmtree(old) if old.is_dir() else old.unlink()
        shutil.copytree(ROOT / "web" / "dist", site, dirs_exist_ok=True)
    run("vet", ["go", "vet", "./..."], cwd=b)
    run("go test", ["go", "test", "./..."], cwd=b)  # testcontainers-go needs Docker running
    subprocess.run(exe(["docker", "rm", "-f", PG_CONTAINER]), capture_output=True)
    run("postgres", ["docker", "run", "-d", "--rm", "--name", PG_CONTAINER, "-e", "POSTGRES_PASSWORD=verify",
                     "-p", "55432:5432", PG_IMAGE])
    url = "http://localhost:8080"
    env = {**os.environ, "DATABASE_URL": "postgres://postgres:verify@localhost:55432/postgres?sslmode=disable",
           "APP_ORIGIN": url, "PORT": "8080", "MAIL_MODE": "capture", **FAKES}
    server = None
    idp = start_fake_idp()
    try:
        for _ in range(60):  # Postgres needs a few seconds before it accepts connections
            # -h 127.0.0.1 checks TCP: during first init the image runs a socket-only server, then restarts
            if subprocess.run(exe(["docker", "exec", PG_CONTAINER, "pg_isready", "-h", "127.0.0.1", "-U", "postgres"]), capture_output=True).returncode == 0:
                break
            time.sleep(1)
        run("migrate", ["go", "run", "./cmd/migrate", "up"], cwd=b, env=env)
        # Build first: `go run` compiles inside the health-check timeout, which fails on a busy machine
        exe_name = "server.exe" if os.name == "nt" else "server"
        run("build server", ["go", "build", "-o", f".bin/{exe_name}", "./cmd/server"], cwd=b)
        server = start([str(b / ".bin" / exe_name)], cwd=b, env=env)
        wait_healthy(url, server)
        (tests or contract)(url, filters)
    finally:
        if server:
            stop(server)
        stop(idp)
        subprocess.run(exe(["docker", "rm", "-f", PG_CONTAINER]), capture_output=True)


if __name__ == "__main__":
    targets = {
        "checks": lambda _: checks(),
        "a": backend_a,
        "b": backend_b,
        "web": web,
        "e2e-a": lambda f: backend_a([], tests=lambda url, _: e2e(url, f)),
        "e2e-b": lambda f: backend_b([], tests=lambda url, _: e2e(url, f)),
    }
    if len(sys.argv) < 2 or sys.argv[1] not in targets:
        sys.exit(__doc__)
    targets[sys.argv[1]](sys.argv[2:])
    print("\nverify: PASS")
