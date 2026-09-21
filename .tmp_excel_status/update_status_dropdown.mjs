import fs from "node:fs/promises";
import { FileBlob, SpreadsheetFile } from "@oai/artifact-tool";

const inputPath = "D:/桌面/问题处理.xlsx";
const outputDir = "D:/桌面/gitai/superagent/outputs/status_dropdown";
const outputPath = `${outputDir}/问题处理_已设置状态下拉.xlsx`;

const input = await FileBlob.load(inputPath);
const workbook = await SpreadsheetFile.importXlsx(input);
const sheet = workbook.worksheets.getItem("Sheet1");
const statusRange = sheet.getRange("G2:G1000");
sheet.getRange("G2").values = [["正常"]];

statusRange.dataValidation = null;
statusRange.dataValidation = {
  rule: {
    type: "list",
    values: ["正常", "处理中", "已处理", "异常", "已闭合", "重新处理"],
  },
};
statusRange.conditionalFormats.deleteAll();

const statusStyles = [
  ["正常", "#E2F0D9", "#006100"],
  ["处理中", "#FFF2CC", "#9C6500"],
  ["已处理", "#DDEBF7", "#1F4E78"],
  ["异常", "#FCE4D6", "#C00000"],
  ["已闭合", "#E7E6E6", "#595959"],
  ["重新处理", "#FCE4D6", "#C65911"],
];
for (const [status, fill, color] of statusStyles) {
  statusRange.conditionalFormats.add("cellIs", {
    operator: "equal",
    formula: `"${status}"`,
    format: { fill, font: { color } },
  });
}

workbook.recalculate();

console.log((await workbook.inspect({
  kind: "table",
  range: "Sheet1!A1:I8",
  include: "values,formulas",
  tableMaxRows: 8,
  tableMaxCols: 9,
})).ndjson);
const preview = await workbook.render({ sheetName: "Sheet1", range: "A1:I8", scale: 2, format: "png" });
await fs.mkdir(outputDir, { recursive: true });
await fs.writeFile(`${outputDir}/preview.png`, new Uint8Array(await preview.arrayBuffer()));
const output = await SpreadsheetFile.exportXlsx(workbook);
await output.save(outputPath);
console.log(outputPath);
