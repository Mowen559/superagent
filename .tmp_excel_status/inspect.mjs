import { FileBlob, SpreadsheetFile } from '@oai/artifact-tool';
const input = await FileBlob.load('D:/桌面/问题处理.xlsx');
const wb = await SpreadsheetFile.importXlsx(input);
console.log((await wb.inspect({kind:'workbook,sheet,table', maxChars:6000, tableMaxRows:12, tableMaxCols:12, tableMaxCellChars:100})).ndjson);
for (const s of wb.worksheets.items) {
 console.log('SHEET:'+s.name);
 console.log((await wb.inspect({kind:'region',sheetId:s.sheetId,range:'A1:Z100',maxChars:12000})).ndjson);
}
