import { Message } from './types/message';
import {
  getSessionHistory,
  listSessions,
  SessionInfo,
  Message as ApiMessage,
  SessionMetadata,
} from './api';
import { convertApiMessageToFrontendMessage } from './components/context_management';
import { getApiUrl } from './config';

// Helper function to ensure working directory is set
export function ensureWorkingDir(metadata: Partial<SessionMetadata>): SessionMetadata {
  return {
    description: metadata.description || '',
    message_count: metadata.message_count || 0,
    total_tokens: metadata.total_tokens || null,
    working_dir: metadata.working_dir || process.env.HOME || '',
    accumulated_input_tokens: metadata.accumulated_input_tokens || null,
    accumulated_output_tokens: metadata.accumulated_output_tokens || null,
    accumulated_total_tokens: metadata.accumulated_total_tokens || null,
  };
}

export interface Session {
  id: string;
  path: string;
  modified: string;
  metadata: SessionMetadata;
}

export interface SessionDetails {
  sessionId: string;
  metadata: SessionMetadata;
  messages: Message[];
}

/**
 * Generate a session ID in the format yyyymmdd_hhmmss
 */
export function generateSessionId(): string {
  const now = new Date();
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, '0');
  const day = String(now.getDate()).padStart(2, '0');
  const hours = String(now.getHours()).padStart(2, '0');
  const minutes = String(now.getMinutes()).padStart(2, '0');
  const seconds = String(now.getSeconds()).padStart(2, '0');

  return `${year}${month}${day}_${hours}${minutes}${seconds}`;
}

/**
 * Fetches all available sessions from the API
 * @returns Promise with sessions data
 */
/**
 * Fetches all available sessions from the API
 * @returns Promise with an array of Session objects
 */
export async function fetchSessions(): Promise<Session[]> {
  const response = await listSessions<true>();

  // Check if the response has the expected structure
  if (response && response.data && response.data.sessions) {
    // Since the API returns SessionInfo, we need to convert to Session
    const sessions = response.data.sessions
      .filter(
        (sessionInfo: SessionInfo) => sessionInfo.metadata && sessionInfo.metadata.message_count > 0
      )
      .map(
        (sessionInfo: SessionInfo): Session => ({
          id: sessionInfo.id,
          path: sessionInfo.path,
          modified: sessionInfo.modified,
          metadata: ensureWorkingDir(sessionInfo.metadata),
        })
      );

    // order sessions by 'modified' date descending
    sessions.sort(
      (a: Session, b: Session) => new Date(b.modified).getTime() - new Date(a.modified).getTime()
    );

    return sessions;
  } else {
    throw new Error('Unexpected response format from listSessions');
  }
}

/**
 * Fetches details for a specific session
 * @param sessionId The ID of the session to fetch
 * @returns Promise with session details
 */
export async function fetchSessionDetails(sessionId: string): Promise<SessionDetails> {
  const response = await getSessionHistory<true>({
    path: { session_id: sessionId },
  });

  // Convert the SessionHistoryResponse to a SessionDetails object
  return {
    sessionId: response.data.sessionId,
    metadata: ensureWorkingDir(response.data.metadata),
    messages: response.data.messages.map((message: ApiMessage) =>
      convertApiMessageToFrontendMessage(message, true, true)
    ), // slight diffs between backend and frontend Message obj
  };
}

/**
 * Updates the metadata for a specific session
 * @param sessionId The ID of the session to update
 * @param description The new description (name) for the session
 * @returns Promise that resolves when the update is complete
 */
export async function updateSessionMetadata(sessionId: string, description: string): Promise<void> {
  const url = getApiUrl(`/sessions/${sessionId}/metadata`);
  const secretKey = await window.electron.getSecretKey();

  const response = await fetch(url, {
    method: 'PUT',
    headers: {
      'Content-Type': 'application/json',
      'X-Secret-Key': secretKey,
    },
    body: JSON.stringify({ description }),
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`Failed to update session metadata: ${response.statusText} - ${errorText}`);
  }
}

/**
 * Resumes a session. Currently, this opens a new window with the session loaded.
 */
export function resumeSession(session: SessionDetails | Session) {
  const resumedSessionId = 'sessionId' in session ? session.sessionId : session.id;
  console.log('Launching session in new window:', resumedSessionId);
  const workingDir = session.metadata?.working_dir;
  if (!workingDir) {
    throw new Error('Cannot resume session: working directory is missing in session metadata');
  }

  window.electron.createChatWindow(
    undefined, // query
    workingDir,
    undefined, // version
    resumedSessionId
  );
}

/**
 * Deletes a specific session
 * @param sessionId The ID of the session to delete
 * @returns Promise that resolves when the deletion is complete
 */
export async function deleteSession(sessionId: string): Promise<void> {
  try {
    const url = getApiUrl(`/sessions/${sessionId}/delete`);
    const secretKey = await window.electron.getSecretKey();

    const response = await fetch(url, {
      method: 'DELETE',
      headers: {
        'X-Secret-Key': secretKey,
      },
    });

    if (!response.ok) {
      const errorText = await response.text();
      throw new Error(`Failed to delete session: ${response.statusText} - ${errorText}`);
    }
  } catch (error) {
    console.error(`Error deleting session ${sessionId}:`, error);
    throw error;
  }
}
