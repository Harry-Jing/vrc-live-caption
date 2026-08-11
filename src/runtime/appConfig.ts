import type { PublicationMode, RecognitionPath } from "./captionPipeline";

export const APP_CONFIG_SCHEMA_VERSION = 1 as const;

export type AppConfig = {
  schemaVersion: typeof APP_CONFIG_SCHEMA_VERSION;
  audio: {
    inputDeviceId: string | null;
  };
  recognition: {
    path: RecognitionPath;
    expectedLanguages: string[];
  };
  osc: {
    host: string;
    port: number;
    enabled: boolean;
  };
  publication: {
    mode: PublicationMode;
  };
  ui: {
    showOngoingPreview: boolean;
  };
};
