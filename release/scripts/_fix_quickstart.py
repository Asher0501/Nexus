import os

path = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'QUICKSTART.md')
path = os.path.normpath(path)

with open(path, 'r', encoding='utf-8') as f:
    content = f.read()

# Find the last ``` code fence before ## 示范工作流
# Delete the blockquote between them
old_start = content.rfind('```', 0, content.find('## 示范工作流'))
if old_start == -1:
    print('ERROR: could not find code fence')
    exit(1)

# Find the section after the code block
after_fence = content[old_start:]

# Find the ## 示范工作流 header
demo_idx = after_fence.find('## 示范工作流')
if demo_idx == -1:
    print('ERROR: could not find ## 示范工作流')
    exit(1)

# Get text between ``` and ## 示范工作流
between = after_fence[3:demo_idx]  # skip the ```
print(f'Text between code fence and header: {repr(between)}')

# Replace: keep ``` then go directly to ## 示范工作流
replacement = '```\n\n## 示范工作流'
content = content[:old_start] + replacement + content[content.find('## 示范工作流') + len('## 示范工作流'):]

with open(path, 'w', encoding='utf-8') as f:
    f.write(content)

print('OK - deleted dependency blockquote')

# Cleanup self
os.unlink(__file__)
