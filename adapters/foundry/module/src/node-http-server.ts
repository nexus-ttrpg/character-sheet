// Node.js HTTP server implementation of RelayHttpServer (§7.1, §13.3).
//
// This is a real HTTP server using Node's built-in `http` module — no
// dependencies. It implements the `RelayHttpServer` interface from types.ts
// so the relay module's endpoint handlers can be exercised over real HTTP in
// integration tests, without a running Foundry instance.
//
// In production, the relay runs inside Foundry's browser context and routes
// are wired to Foundry's Express integration. This NodeHttpServer is for
// testing and standalone relay mode (e.g. a headless relay for CI).
//
// Route patterns use Express-style `:param` syntax:
//   GET  /nexus/actors           → no params
//   GET  /nexus/actors/:id       → { id: "abc" }
//   POST /nexus/actors/:id/apply → { id: "abc" }

import { createServer, type Server, type IncomingMessage, type ServerResponse } from "node:http";
import type { RelayHttpServer, RelayRequest, RelayResponse } from "./types.js";

interface CompiledRoute {
  method: "GET" | "POST";
  /** Segments: literal strings or `:param` markers. */
  segments: Array<{ type: "literal"; value: string } | { type: "param"; name: string }>;
  handler: (req: RelayRequest) => Promise<RelayResponse>;
}

/**
 * Parse a path pattern like `/nexus/actors/:id/apply` into segments.
 */
function compilePattern(path: string): CompiledRoute["segments"] {
  return path.split("/").filter((s) => s.length > 0).map((seg) => {
    if (seg.startsWith(":")) {
      return { type: "param" as const, name: seg.slice(1) };
    }
    return { type: "literal" as const, value: seg };
  });
}

/**
 * Match an incoming URL path against a compiled route pattern.
 * Returns the extracted params, or null if no match.
 */
function matchRoute(
  segments: CompiledRoute["segments"],
  pathParts: string[],
): Record<string, string> | null {
  if (segments.length !== pathParts.length) return null;
  const params: Record<string, string> = {};
  for (let i = 0; i < segments.length; i++) {
    const seg = segments[i]!;
    const part = pathParts[i]!;
    if (seg.type === "literal") {
      if (seg.value !== part) return null;
    } else {
      params[seg.name] = decodeURIComponent(part);
    }
  }
  return params;
}

/**
 * Read and parse the JSON body from an incoming request.
 * Returns undefined if the body is empty or not valid JSON.
 */
function readJsonBody(req: IncomingMessage): Promise<unknown> {
  return new Promise((resolve) => {
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      const raw = Buffer.concat(chunks).toString("utf-8");
      if (raw.length === 0) {
        resolve(undefined);
        return;
      }
      try {
        resolve(JSON.parse(raw));
      } catch {
        resolve(undefined);
      }
    });
    req.on("error", () => resolve(undefined));
  });
}

/**
 * Node.js HTTP server implementing the RelayHttpServer interface.
 *
 * Usage:
 *   const server = new NodeHttpServer();
 *   registerRoutes(server, game, tracker, apiKey);
 *   await server.listen(0); // ephemeral port
 *   // ... make HTTP requests ...
 *   await server.close();
 */
export class NodeHttpServer implements RelayHttpServer {
  private routes: CompiledRoute[] = [];
  private server: Server | null = null;
  private port: number = 0;

  registerRoute(
    method: "GET" | "POST",
    path: string,
    handler: (req: RelayRequest) => Promise<RelayResponse>,
  ): void {
    this.routes.push({
      method,
      segments: compilePattern(path),
      handler,
    });
  }

  /**
   * Start listening on the given port. Use 0 for an ephemeral port.
   * Returns the actual port assigned by the OS.
   */
  listen(port: number = 0): Promise<number> {
    return new Promise((resolve, reject) => {
      this.server = createServer((req, res) => {
        this.handleRequest(req, res).catch((err) => {
          console.error("relay server error:", err);
          if (!res.headersSent) {
            res.writeHead(500, { "content-type": "application/json" });
            res.end(JSON.stringify({ error: "internal server error" }));
          }
        });
      });

      this.server.on("error", reject);
      this.server.listen(port, "127.0.0.1", () => {
        const addr = this.server!.address();
        this.port = typeof addr === "object" && addr ? addr.port : port;
        resolve(this.port);
      });
    });
  }

  /**
   * Stop the server and close all connections.
   */
  close(): Promise<void> {
    return new Promise((resolve) => {
      if (!this.server) {
        resolve();
        return;
      }
      this.server.close(() => {
        this.server = null;
        resolve();
      });
    });
  }

  /** The port the server is listening on (0 if not started). */
  get listeningPort(): number {
    return this.port;
  }

  /**
   * Handle a single incoming HTTP request.
   */
  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const url = new URL(req.url ?? "/", `http://127.0.0.1:${this.port}`);
    const pathParts = url.pathname.split("/").filter((s) => s.length > 0);
    const method = req.method ?? "GET";

    // Find the first matching route.
    for (const route of this.routes) {
      if (route.method !== method) continue;
      const params = matchRoute(route.segments, pathParts);
      if (params === null) continue;

      // Found a match — build the RelayRequest and call the handler.
      const body = method === "POST" ? await readJsonBody(req) : undefined;
      const authHeader = req.headers["authorization"];

      const relayReq: RelayRequest = {
        params,
        ...(body !== undefined ? { body } : {}),
        ...(authHeader !== undefined ? { authHeader } : {}),
      };

      const relayRes: RelayResponse = await route.handler(relayReq);

      res.writeHead(relayRes.status, { "content-type": "application/json" });
      res.end(JSON.stringify(relayRes.body));
      return;
    }

    // No route matched.
    res.writeHead(404, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: "not found" }));
  }
}
