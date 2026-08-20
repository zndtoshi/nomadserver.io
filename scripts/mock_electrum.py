#!/usr/bin/env python3
"""Mock Electrum server for nomad-server development.

Speaks just enough of the Electrum protocol (newline-delimited JSON-RPC
over TCP) to exercise the server's handlers end-to-end without a real
node. NOT a test of Bitcoin semantics — all data is canned.

Usage: python3 scripts/mock_electrum.py [port]   (default 50001)
Then run nomad-server with ELECTRS_ADDR=127.0.0.1:50001
"""

import json
import socketserver
import sys
import threading

FAKE_HEADER = "00" * 80  # 80 zero bytes decode as a (nonsense) block header
TIP_HEIGHT = 800_005
HISTORY = [
    {"height": 800_001, "tx_hash": "aa" * 32, "fee": 1000},
    {"height": 800_002, "tx_hash": "bb" * 32, "fee": 1500},
]
UTXOS = [
    {"height": 800_001, "tx_hash": "aa" * 32, "tx_pos": 0, "value": 50_000},
    {"height": 800_002, "tx_hash": "bb" * 32, "tx_pos": 1, "value": 75_000},
]
RAW_TX = "0200000001" + "00" * 41 + "6a"  # arbitrary bytes for tx fetch

# Watcher-test scenario: a new tx appears in the mempool at +20s and
# confirms at +45s. The watcher should emit notify/new_tx twice
# (height 0, then height 800006).
NEW_TX = {"height": 0, "tx_hash": "cc" * 32, "fee": 2000}


def _scenario():
    import time

    time.sleep(20)
    HISTORY.append(NEW_TX)
    time.sleep(25)
    NEW_TX["height"] = TIP_HEIGHT + 1


threading.Thread(target=_scenario, daemon=True).start()


def handle(method, params):
    if method == "server.version":
        return ["mock-electrum 0.1", "1.4"]
    if method == "server.banner":
        return "mock"
    if method == "blockchain.headers.subscribe":
        return {"hex": FAKE_HEADER, "height": TIP_HEIGHT}
    if method == "blockchain.scripthash.get_balance":
        return {"confirmed": 125_000, "unconfirmed": 0}
    if method == "blockchain.scripthash.get_history":
        return HISTORY
    if method == "blockchain.scripthash.listunspent":
        return UTXOS
    if method == "blockchain.block.header":
        return FAKE_HEADER
    if method == "blockchain.estimatefee":
        n = params[0] if params else 1
        return max(0.00001, 0.0001 / n)
    if method == "blockchain.transaction.get":
        return RAW_TX
    if method == "blockchain.transaction.broadcast":
        raise ValueError("mock: broadcast always fails")
    raise ValueError(f"mock: unhandled method {method}")


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        for line in self.rfile:
            try:
                req = json.loads(line.decode())
                resp = {"jsonrpc": "2.0", "id": req.get("id")}
                try:
                    resp["result"] = handle(req["method"], req.get("params", []))
                except ValueError as e:
                    resp["error"] = {"code": -1, "message": str(e)}
                self.wfile.write((json.dumps(resp) + "\n").encode())
            except Exception as e:  # keep the connection alive on bad input
                try:
                    self.wfile.write(
                        (json.dumps({"jsonrpc": "2.0", "id": None,
                                     "error": {"code": -1, "message": str(e)}}) + "\n").encode()
                    )
                except Exception:
                    return


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 50001
    srv = Server(("127.0.0.1", port), Handler)
    print(f"mock electrum on 127.0.0.1:{port}", flush=True)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    threading.Event().wait()
