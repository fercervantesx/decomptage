import type { Socket } from 'bun';
import type net from 'net';

export interface JsonRpcRequest {
  jsonrpc: string;
  id?: number | string | null;
  method: string;
  params?: Record<string, unknown>;
}

export interface JsonRpcResponse {
  jsonrpc: '2.0';
  id?: number | string | null;
  result?: unknown;
  error?: { code: number; message: string };
}

export interface JsonRpcNotification {
  method: string;
  params: Record<string, unknown>;
}

export type ClientSocket = net.Socket;

export function sendResponse(socket: ClientSocket, id: number | string | null | undefined, result: unknown) {
  const msg: JsonRpcResponse = { jsonrpc: '2.0', id: id ?? null, result };
  socket.write(JSON.stringify(msg) + '\n');
}

export function sendError(socket: ClientSocket, id: number | string | null | undefined, code: number, message: string) {
  const msg: JsonRpcResponse = { jsonrpc: '2.0', id: id ?? null, error: { code, message } };
  socket.write(JSON.stringify(msg) + '\n');
}

export function sendNotification(socket: ClientSocket, method: string, params: Record<string, unknown>) {
  const msg: JsonRpcNotification = { method, params };
  socket.write(JSON.stringify(msg) + '\n');
}
