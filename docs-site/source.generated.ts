/// <reference types="vite/client" />
import { fromConfig } from 'fumadocs-mdx/runtime/vite';
import type * as Config from './source.config';

export const create = fromConfig<typeof Config>();

export const docs = {
  doc: create.doc("docs", "./../docs", import.meta.glob(["./**/*.{mdx,md}"], {
    "base": "./../docs",
    "query": {
      "collection": "docs"
    }
  })),
  meta: create.meta("docs", "./../docs", import.meta.glob(["./**/*.{json,yaml}"], {
    "import": "default",
    "base": "./../docs",
    "query": {
      "collection": "docs"
    }
  }))
};