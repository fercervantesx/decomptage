import { tool } from 'ai';
import { z } from 'zod';

export const suggestCommandTool = tool({
  description: 'Suggest a shell command for the user to run. The command appears as an actionable card. The user decides whether to insert it into their terminal.',
  parameters: z.object({
    command: z.string().describe('The shell command to suggest'),
    explanation: z.string().describe('Brief explanation of what this command does'),
    danger_level: z.enum(['safe', 'caution', 'destructive']).optional().describe("How dangerous: 'destructive' for rm -rf, git reset --hard, etc."),
  }),
  execute: async ({ command, explanation, danger_level }) => {
    // This is handled specially by the agent — emits a notification
    return `Suggested command to user: ${command}`;
  },
});
