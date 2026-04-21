export interface ColumnInfo { name: string; type: string; nullable: boolean; }
export interface TableDoc  { name: string; columns: ColumnInfo[]; }
export interface DatabaseDoc { name: string; tables: TableDoc[]; }
export interface SchemaDoc { databases: DatabaseDoc[]; }

export async function fetchSchema(args: { endpoint: string; token: string }): Promise<SchemaDoc> {
  const res = await fetch(`${args.endpoint.replace(/\/$/, "")}/v1/catalog/schema`, {
    headers: { authorization: `Bearer ${args.token}` },
  });
  if (res.status === 401 || res.status === 403) throw new Error(`unauthorized (${res.status})`);
  if (!res.ok) throw new Error(`catalog fetch failed: ${res.status}`);
  return (await res.json()) as SchemaDoc;
}
