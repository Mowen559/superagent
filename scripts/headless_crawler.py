import sys
import json
import asyncio
from playwright.async_api import async_playwright

async def main():
    if len(sys.argv) < 2:
        print(json.dumps({"error": "Missing URL"}))
        return
        
    url = sys.argv[1]
    
    try:
        async with async_playwright() as p:
            browser = await p.chromium.launch(headless=True)
            page = await browser.new_page()
            await page.goto(url, wait_until="networkidle", timeout=30000)
            
            title = await page.title()
            content = await page.content()
            
            # Very basic text extraction
            text_content = await page.evaluate("document.body.innerText")
            
            result = {
                "title": title,
                "text": text_content[:50000], # truncate to 50k chars
                "success": True
            }
            
            await browser.close()
            print(json.dumps(result))
            
    except Exception as e:
        print(json.dumps({"error": str(e), "success": False}))

if __name__ == "__main__":
    asyncio.run(main())

