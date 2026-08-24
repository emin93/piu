/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_PIU_MODEL_QA_GALLERY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
