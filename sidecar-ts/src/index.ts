import net from 'net';
import { unlinkSync, existsSync, mkdirSync } from 'fs';
import { join } from 'path';
import { homedir } from 'os';
import { sendResponse, sendError, sendNotification, type ClientSocket, type JsonRpcRequest } from './protocol';
import { getAvailableProviders } from './providers';
import { runAgent } from './agent';
import { setSocket as setRunCommandSocket, resolvePendingApproval, resolvePendingResult } from './tools/run-command';
import { setSocket as setReadTerminalSocket, resolvePendingRead } from './tools/read-terminal';

// Socket path
const SOCKET_DIR = join(homedir(), 'Library', 'Caches', 'decomptage');
const SOCKET_PATH = join(SOCKET_DIR, 'sidecar.sock');

// Per-connection state
interface Session {
  contextBuffer: string;
  conversationHistory: Array<{ role: string; content: string }>;
}

const sessions = new Map<ClientSocket, Session>();

function getSession(socket: ClientSocket): Session {
  if (!sessions.has(socket)) {
    sessions.set(socket, { contextBuffer: '', conversationHistory: [] });
  }
  return sessions.get(socket)!;
}

async function handleRequest(socket: ClientSocket, req: JsonRpcRequest) {
  const session = getSession(socket);

  switch (req.method) {
    case 'session.hello': {
      const providers = await getAvailableProviders();
      sendResponse(socket, req.id, {
        session_id: crypto.randomUUID(),
        providers,
        features: ['attachments', 'streaming', 'context', 'tools'],
      });
      break;
    }

    case 'context.push': {
      const text = (req.params?.payload as any)?.text as string;
      if (text) {
        const kind = req.params?.kind as string;
        if (kind === 'full_screen') {
          session.contextBuffer = text;
        } else {
          session.contextBuffer += '\n---\n' + text;
          // Cap at ~50KB
          if (session.contextBuffer.length > 50000) {
            session.contextBuffer = session.contextBuffer.slice(-50000);
          }
        }
      }
      sendResponse(socket, req.id, { status: 'ok' });
      break;
    }

    case 'settings.update': {
      if (req.params) {
        for (const [key, value] of Object.entries(req.params)) {
          if (typeof value === 'string' && value) {
            process.env[key] = value;
          }
        }
      }
      sendResponse(socket, req.id, { status: 'ok' });
      break;
    }

    case 'chat.send': {
      sendResponse(socket, req.id, { stream_id: crypto.randomUUID(), status: 'streaming' });

      const providerId = (req.params?.provider as string) ?? 'anthropic';
      const model = (req.params?.model as string) ?? 'claude-sonnet-4-6';
      const maxTokens = (req.params?.max_tokens as number) ?? 4096;

      // Parse messages from client
      const clientMessages = (req.params?.messages as any[]) ?? [];
      const messages = clientMessages.map(m => ({
        role: m.role as 'user' | 'assistant',
        content: typeof m.content === 'string' ? m.content : JSON.stringify(m.content),
      }));

      // Keep bottom 80 lines of context
      let context = session.contextBuffer;
      if (context) {
        const lines = context.split('\n');
        if (lines.length > 80) {
          context = lines.slice(-80).join('\n');
        }
      }

      // Set sockets for interactive tools
      setRunCommandSocket(socket);
      setReadTerminalSocket(socket);

      await runAgent({
        providerId,
        model,
        messages,
        terminalContext: context || undefined,
        socket,
        maxTokens,
      });
      break;
    }

    // Client responses to tool requests
    case 'chat.tool_approval_response': {
      const id = req.params?.tool_call_id as string;
      const approved = req.params?.approved as boolean;
      if (id) resolvePendingApproval(id, approved);
      break;
    }

    case 'chat.run_command_result': {
      const id = req.params?.tool_call_id as string;
      const output = req.params?.output as string ?? '';
      const exitCode = req.params?.exit_code as number ?? -1;
      if (id) resolvePendingResult(id, output, exitCode);
      break;
    }

    case 'chat.read_terminal_response': {
      const id = req.params?.request_id as string;
      const text = req.params?.text as string ?? '';
      if (id) resolvePendingRead(id, text);
      break;
    }

    default:
      sendError(socket, req.id, -32601, `Method not found: ${req.method}`);
  }
}

// Start UDS server
mkdirSync(SOCKET_DIR, { recursive: true });
if (existsSync(SOCKET_PATH)) unlinkSync(SOCKET_PATH);

const server = net.createServer((socket) => {
  console.log('[sidecar] client connected');

  let buffer = '';

  socket.on('data', (data) => {
    buffer += data.toString();
    let newlineIdx: number;
    while ((newlineIdx = buffer.indexOf('\n')) !== -1) {
      const line = buffer.slice(0, newlineIdx).trim();
      buffer = buffer.slice(newlineIdx + 1);
      if (!line) continue;

      try {
        const req = JSON.parse(line) as JsonRpcRequest;
        handleRequest(socket, req).catch(err => {
          console.error('[sidecar] handler error:', err);
          sendError(socket, req.id, -32603, err.message);
        });
      } catch (e) {
        sendError(socket, null, -32700, 'Parse error');
      }
    }
  });

  socket.on('close', () => {
    console.log('[sidecar] client disconnected');
    sessions.delete(socket);
  });

  socket.on('error', (err) => {
    console.error('[sidecar] socket error:', err.message);
  });
});

server.listen(SOCKET_PATH, () => {
  console.log(`[sidecar] listening on ${SOCKET_PATH}`);
});

process.on('SIGINT', () => {
  server.close();
  if (existsSync(SOCKET_PATH)) unlinkSync(SOCKET_PATH);
  process.exit(0);
});

process.on('SIGTERM', () => {
  server.close();
  if (existsSync(SOCKET_PATH)) unlinkSync(SOCKET_PATH);
  process.exit(0);
});
