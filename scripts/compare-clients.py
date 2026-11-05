#!/usr/bin/env python3
"""Compare load clients against one server that is already running.

Each case is run by every client in turn, for several rounds, and the median
is reported: completed units per second, client CPU per unit (user, system),
context switches per unit and failed requests. A unit is a request, or an
established connection in the connection test.

The server is the caller's: start it, pass its address, stop it afterwards.
This measures clients. It says nothing about the server's capacity.

    scripts/compare-clients.py --plain 127.0.0.1:8080 \\
        --tls 127.0.0.1:8443 --cacert certs/ca/bench/ca.crt \\
        old=./brk-before new=./target/release/brk wrk=/usr/bin/wrk

A client named `wrk` runs only the plain cases without pipelining.
"""

import argparse
import re
import resource
import statistics
import subprocess
import sys


def arguments():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--plain", required=True, metavar="ADDRESS",
                        help="address of the cleartext listener")
    parser.add_argument("--tls", metavar="ADDRESS",
                        help="address of the TLS listener; TLS cases need it")
    parser.add_argument("--cacert", metavar="FILE",
                        help="authority the TLS listener's certificate chains to")
    parser.add_argument("--host", default="benchmark.test",
                        help="name in the TLS certificate (default: %(default)s)")
    parser.add_argument("--path", default="/plain")
    parser.add_argument("--threads", type=int, required=True,
                        help="client threads, the same for every client")
    parser.add_argument("--seconds", type=int, required=True,
                        help="length of one run")
    parser.add_argument("--rounds", type=int, required=True,
                        help="runs of each client per case")
    parser.add_argument("--timeout", default="100ms",
                        help="request deadline (default: %(default)s)")
    parser.add_argument("--connections", type=int, nargs="+",
                        default=[8, 64, 256],
                        help="connection counts of the depth-1 cases")
    parser.add_argument("--depths", type=int, nargs="+", default=[8, 32],
                        help="pipeline depths, at the largest connection count")
    parser.add_argument("--owners", type=int, default=32,
                        help="concurrent connections in the connection test")
    parser.add_argument("clients", nargs="+", metavar="NAME=BINARY")
    return parser.parse_args()


def cases(options):
    def target(secure):
        if secure:
            return [f"https://{options.host}{options.path}",
                    "--connect", options.tls, "--cacert", options.cacert]
        return [f"http://{options.plain}{options.path}"]

    common = ["--threads", str(options.threads),
              "--phase", f"{options.seconds}s", "--timeout", options.timeout]

    def requests(secure, connections, depth):
        level = str(depth)
        return target(secure) + common + [
            "--conn", str(connections),
            "--start", level, "--pace", level, "--max", level]

    def connections(secure, extra=()):
        level = str(options.owners)
        return ["conn"] + target(secure) + common + [
            "--start", level, "--pace", level, "--max", level,
            "--requests", "1", *extra]

    most = max(options.connections)
    listed = [(f"plain c={count} depth 1", requests(False, count, 1), "successful")
              for count in options.connections]
    listed += [(f"plain c={most} depth {depth}",
                requests(False, most, depth), "successful")
               for depth in options.depths]
    listed.append((f"conn plain {options.owners}", connections(False), "established"))
    if options.tls and options.cacert:
        listed.append((f"tls c={most} depth 1",
                       requests(True, most, 1), "successful"))
        listed += [(f"tls c={most} depth {depth}",
                    requests(True, most, depth), "successful")
                   for depth in options.depths[:1]]
        listed.append((f"conn tls resumed {options.owners}",
                       connections(True), "established"))
        listed.append((f"conn tls full {options.owners}",
                       connections(True, ["--no-resume"]), "established"))
    return listed


def as_wrk(command, options):
    """The same case for wrk, or None where wrk has no equivalent."""
    if command[0] == "conn" or "--connect" in command:
        return None
    if command[command.index("--start") + 1] != "1":
        return None
    return [f"-t{options.threads}",
            f"-c{command[command.index('--conn') + 1]}",
            f"-d{options.seconds}s", command[0]]


def run(name, binary, command, unit, options):
    if name == "wrk":
        command = as_wrk(command, options)
        if command is None:
            return None, None
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    done = subprocess.run([binary, *command], capture_output=True, text=True)
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    text = done.stdout + done.stderr
    if name == "wrk":
        count = sum(int(n) for n in re.findall(r"(\d+) requests in", text))
        failed = sum(int(n) for n in re.findall(r"timeout (\d+)", text))
    else:
        count = sum(int(n) for n in re.findall(rf"\b{unit}=(\d+)", text))
        failed = sum(int(n) for n in re.findall(r"\berrors=(\d+)", text))
    if count == 0:
        return None, text[-400:]
    user = (after.ru_utime - before.ru_utime) / count * 1e6
    system = (after.ru_stime - before.ru_stime) / count * 1e6
    switches = (after.ru_nvcsw - before.ru_nvcsw
                + after.ru_nivcsw - before.ru_nivcsw) / count
    return (count / options.seconds, user, system, switches, failed), None


def main():
    options = arguments()
    clients = [client.split("=", 1) for client in options.clients]
    # Many connections need many descriptors; take what the limit allows.
    _, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    wanted = 65536 if hard == resource.RLIM_INFINITY else min(65536, hard)
    resource.setrlimit(resource.RLIMIT_NOFILE, (wanted, hard))
    print(f"{'case':26}{'client':10}{'per s':>10}{'user us':>9}"
          f"{'sys us':>8}{'total':>8}{'switch':>8}{'failed':>8}")
    for label, command, unit in cases(options):
        samples = {name: [] for name, _ in clients}
        for _ in range(options.rounds):
            for name, binary in clients:
                result, problem = run(name, binary, command, unit, options)
                if result is not None:
                    samples[name].append(result)
                elif problem is not None:
                    print(f"{label:26}{name:10} no result: {problem!r}")
        for name, _ in clients:
            if not samples[name]:
                continue
            rate, user, system, switches, _ = (
                statistics.median(column) for column in zip(*samples[name]))
            failed = sum(sample[4] for sample in samples[name])
            print(f"{label:26}{name:10}{rate:10.0f}{user:9.2f}{system:8.2f}"
                  f"{user + system:8.2f}{switches:8.3f}{failed:8.0f}")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
