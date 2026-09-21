import fs from "node:fs/promises";
import { FileBlob, SpreadsheetFile } from "@oai/artifact-tool";

const input = await FileBlob.load("D:/桌面/问题处理.xlsx");
const workbook = await SpreadsheetFile.importXlsx(input);
const preview = await workbook.render({ sheetName: "Sheet1", range: "A1:I8", scale: 2, format: "png" });
await fs.writeFile("D:/桌面/gitai/superagent/.tmp_excel_status/source.png", new Uint8Array(await preview.arrayBuffer()));
