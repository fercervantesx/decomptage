import { tool } from 'ai';
import { z } from 'zod';
import type { ClientSocket } from '../protocol';
import { sendNotification } from '../protocol';

let _socket: ClientSocket | null = null;
let _pendingReads: Map<string, (text: string) => void> = new Map();

export function setSocket(socket: ClientSocket) { _socket = socket; }
export function resolvePendingRead(id: string, text: string) {
  _pendingReads.get(id)?.(text);
  _pendingReads.delete(id);
}

export const readTerminalTool = tool({
  description: 'Read the current visible content of the user\'s terminal screen.',
  parameters: z.object({
    lines: z.number().optional().describe('Number of lines to read. Omit for full screen.'),
  }),
  execute: async ({ lines }) => {
    if (!_socket) return 'Error: not connected to terminal';

    const requestId = crypto.randomUUID();

    sendNotification(_socket, 'chat.read_terminal_request', { request_id: requestId });

    const text = await new Promise<string>((resolve) => {
      _pendingReads.set(requestId, resolve);
      setTimeout(() => { resolve('Timeout reading terminal'); _pendingReads.delete(requestId); }, 5000);
    });

    if (lines && text) {
      const allLines = text.split('\n');
      return allLines.slice(-lines).join('\n');
    }
    return text;
  },
});
