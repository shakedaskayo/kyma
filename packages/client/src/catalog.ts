import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

export interface ColumnInfo { name: string; type: string; nullable: boolean; }
export interface TableDoc  { name: string; columns: ColumnInfo[]; }
export interface DatabaseDoc { name: string; tables: TableDoc[]; }
export interface SchemaDoc { databases: DatabaseDoc[]; }

export async function fetchSchema(t: KymaTransport): Promise<SchemaDoc> {
  const res = await t.request("/v1/catalog/schema");
  if (!res.ok) throw await errorFromResponse(res);
  return (await res.json()) as SchemaDoc;
}
