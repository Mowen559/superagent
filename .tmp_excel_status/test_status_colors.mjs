import fs from "node:fs/promises";
import { FileBlob, SpreadsheetFile } from "@oai/artifact-tool";
const input = await FileBlob.load("D:/桌面/gitai/superagent/outputs/status_dropdown/问题处理_已设置状态下拉.xlsx");
const workbook = await SpreadsheetFile.importXlsx(input);
const sheet = workbook.worksheets.getItem("Sheet1");
sheet.getRange("G3:G8").values = [["处理中"],["已处理"],["异常"],["已闭合"],["重新处理"] ,[null]];
const preview = await workbook.render({ sheetName: "Sheet1", range: "F1:G8", scale: 2, format: "png" });
await fs.writeFile("D:/桌面/gitai/superagent/.tmp_excel_status/test_colors.png", new Uint8Array(await preview.arrayBuffer()));
