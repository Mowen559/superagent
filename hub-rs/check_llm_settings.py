with open(r'd:\mowen\deepcloud\super-agent\hub-rs\src\config\llm_settings.rs', 'r', encoding='utf-8') as f:
    for line in f:
        if 'json' in line or 'path' in line or 'config' in line:
            print(line.strip())
