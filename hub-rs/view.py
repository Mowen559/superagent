with open('src/services/visual_pipeline/native_cropper.rs', 'r', encoding='utf-8') as f:
    lines = f.readlines()
with open('view.txt', 'w', encoding='utf-8') as f:
    for i, line in enumerate(lines):
        if i >= 365 and i <= 475:
            f.write(f"{i+1:03d} | {line}")
