import { FileBlob, SpreadsheetFile } from "@oai/artifact-tool";
const input = await FileBlob.load("D:/桌面/问题处理.xlsx");
const workbook = await SpreadsheetFile.importXlsx(input);
console.log(workbook.help("range.dataValidation", { include: "index,examples,notes", maxChars: 4000 }).ndjson);
