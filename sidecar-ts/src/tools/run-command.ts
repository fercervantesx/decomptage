import { tool } from 'ai';
import { z } from 'zod';
import type { ClientSocket } from '../protocol';
import { sendNotification } from '../protocol';

// These are set by the agent when it creates the tool instances
let _socket: ClientSocket | null = null;
let _pendingApprovals: Map<string, (approved: boolean) => void> = new Map();
let _pendingResults: Map<string, (result: { output: string; exitCode: number }) => void> = new Map();

export function setSocket(socket: ClientSocket) { _socket = socket; }
export function resolvePendingApproval(id: string, approved: boolean) {
  _pendingApprovals.get(id)?.(approved);
  _pendingApprovals.delete(id);
}
export function resolvePendingResult(id: string, output: string, exitCode: number) {
  _pendingResults.get(id)?.({ output, exitCode });
  _pendingResults.delete(id);
}

export const runCommandTool = tool({
  description: 'Execute a shell command in the user\'s terminal. Requires user approval. The output will be returned to you.',
  parameters: z.object({
    command: z.string().describe('The command to execute'),
    explanation: z.string().describe('Why you need to run this command'),
  }),
  execute: async ({ command, explanation }) => {
    if (!_socket) return 'Error: not connected to terminal';

    const toolCallId = crypto.randomUUID();

    // Request approval
    sendNotification(_socket, 'chat.tool_approval_request', {
      tool_call_id: toolCallId,
      tool_name: 'run_command',
      command,
      explanation,
      danger_level: classifyDanger(command),
    });

    // Wait for approval
    const approved = await new Promise<boolean>((resolve) => {
      _pendingApprovals.set(toolCallId, resolve);
      // Timeout after 60s
      setTimeout(() => { resolve(false); _pendingApprovals.delete(toolCallId); }, 60000);
    });

    if (!approved) return 'User denied this command.';

    // Wait for command result
    const result = await new Promise<{ output: string; exitCode: number }>((resolve) => {
      _pendingResults.set(toolCallId, resolve);
      setTimeout(() => { resolve({ output: 'Timeout waiting for output', exitCode: -1 }); _pendingResults.delete(toolCallId); }, 30000);
    });

    return `Exit code: ${result.exitCode}\nOutput:\n${result.output}`;
  },
});

function classifyDanger(command: string): string {
  const lower = command.toLowerCase();
  const destructive = ['rm -rf', 'rm -r', 'git reset --hard', 'git push --force', 'git push -f', 'mkfs', 'dd if='];
  const caution = ['sudo', 'kill', 'pkill', 'docker rm', 'npm uninstall'];
  if (destructive.some(p => lower.includes(p))) return 'destructive';
  if (caution.some(p => lower.includes(p))) return 'caution';
  return 'safe';
}
