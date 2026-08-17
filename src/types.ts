export type ProviderProtocol =
  | "openai_responses"
  | "openai_chat"
  | "anthropic_messages";

export interface ProviderSummary {
  id: string;
  name: string;
  apiUrl: string;
  modelCount: number;
  hasApiKey: boolean;
  active: boolean;
  protocol: ProviderProtocol | string;
  routingMode: "direct" | "local" | string;
}

export interface ProviderState {
  providers: ProviderSummary[];
  activeProviderId: string | null;
  officialActive: boolean;
}

export interface SavedProvider {
  provider: ProviderSummary;
  routingRequired: boolean;
  routingMessage: string | null;
}

export interface ProviderSyncReport {
  rolloutFilesUpdated: number;
  sqliteRowsUpdated: number;
  backupPath: string;
}

export interface ProviderDraft {
  name: string;
  apiUrl: string;
  apiKey: string;
}
