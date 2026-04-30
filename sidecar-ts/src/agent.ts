import { streamText, type CoreMessage } from 'ai';
import { getProvider } from './providers';
import { sendNotification, type ClientSocket } from './protocol';
import { readFileTool } from './tools/read-file';
import { writeFileTool } from './tools/write-file';
import { editFileTool } from './tools/edit-file';
import { listDirectoryTool } from './tools/list-directory';
import { searchFilesTool } from './tools/search-files';
import { webFetchTool } from './tools/web-fetch';
import { suggestCommandTool } from './tools/suggest-command';
import { runCommandTool } from './tools/run-command';
import { readTerminalTool } from './tools/read-terminal';

const SYSTEM_PROMPT = `You are a senior developer assistant embedded in a terminal emulator called Decomptage. You have full access to the user's filesystem, can read/write files, search code, execute commands, and fetch web documentation.

Your capabilities:
- read_file: Read any file (source code, docs, configs)
- write_file: Create or overwrite files
- edit_file: Make targeted edits to existing files
- list_directory: Browse the filesystem
- search_files: Grep across the codebase
- web_fetch: Read documentation from URLs
- read_terminal: See what's currently on the user's terminal screen
- run_command: Execute commands in the terminal (requires user approval)
- suggest_command: Suggest a command for the user to run manually

When guiding the user through tasks:
- Be concise but thorough
- Use tools proactively to understand the project before suggesting actions
- Suggest one step at a time for complex workflows
- Use suggest_command for commands the user should review before running
- Use run_command for safe diagnostic commands you need output from

When you see terminal context, it shows what's currently on the user's screen. Use it to understand their situation without them having to explain.`;

export interface AgentOptions {
  providerId: string;
  model: string;
  messages: CoreMessage[];
  terminalContext?: string;
  socket: ClientSocket;
  maxTokens?: number;
}

export async function runAgent(options: AgentOptions) {
  const { providerId, model: modelId, messages, terminalContext, socket, maxTokens } = options;

  // Prepend terminal context as a user message if available
  const allMessages: CoreMessage[] = [];
  if (terminalContext) {
    allMessages.push({
      role: 'user',
      content: `[Terminal context — this is what's currently visible in my terminal:]\n\n\`\`\`\n${terminalContext}\n\`\`\``,
    });
  }
  allMessages.push(...messages);

  const allTools = {
    read_file: readFileTool,
    write_file: writeFileTool,
    edit_file: editFileTool,
    list_directory: listDirectoryTool,
    search_files: searchFilesTool,
    web_fetch: webFetchTool,
    suggest_command: suggestCommandTool,
    run_command: runCommandTool,
    read_terminal: readTerminalTool,
  };

  try {
    const provider = getProvider(providerId, modelId);

    const result = streamText({
      model: provider,
      system: SYSTEM_PROMPT,
      messages: allMessages,
      tools: allTools,
      maxSteps: 10,
      maxTokens: maxTokens ?? 4096,
      onStepFinish: ({ toolCalls, toolResults }) => {
        // Notify client about tool calls for UI rendering
        for (const tc of toolCalls ?? []) {
          if (tc.toolName === 'suggest_command') {
            sendNotification(socket, 'chat.suggested_command', {
              command: tc.args.command,
              explanation: tc.args.explanation,
              danger_level: tc.args.danger_level ?? 'safe',
              source: 'tool_call',
            });
          }
        }
      },
    });

    // Stream text to client
    for await (const chunk of (await result).textStream) {
      if (chunk) {
        sendNotification(socket, 'chat.delta', { text: chunk });
      }
    }

    const finalResult = await result;
    sendNotification(socket, 'chat.done', {
      usage: {
        input_tokens: finalResult.usage?.promptTokens ?? 0,
        output_tokens: finalResult.usage?.completionTokens ?? 0,
      },
    });
  } catch (error: any) {
    sendNotification(socket, 'chat.error', { message: error.message ?? String(error) });
  }
}
