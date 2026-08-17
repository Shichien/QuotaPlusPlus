import { invoke } from "@tauri-apps/api/core";
import type {
  ProviderDraft,
  ProviderState,
  ProviderSyncReport,
  SavedProvider,
} from "../types";

export const cswitchApi = {
  listProviders: () => invoke<ProviderState>("list_providers"),
  saveProvider: (providerId: string | null, draft: ProviderDraft) =>
    invoke<SavedProvider>("save_provider", {
      providerId,
      name: draft.name,
      apiUrl: draft.apiUrl,
      apiKey: draft.apiKey,
    }),
  activateProvider: (providerId: string) =>
    invoke<ProviderSyncReport>("activate_provider", { providerId }),
  enableProviderRouting: (providerId: string) =>
    invoke<void>("enable_provider_routing", { providerId }),
  deleteProvider: (providerId: string) =>
    invoke<void>("delete_provider", { providerId }),
  startOfficialLogin: () =>
    invoke<ProviderSyncReport>("start_official_login"),
  cancelOfficialLogin: () => invoke<void>("cancel_official_login"),
};
