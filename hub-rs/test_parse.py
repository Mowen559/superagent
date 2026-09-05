import json

accumulated_raw = """event: response.created
data: {"type":"response.created","response":{"id":"resp_04cbc44a7286b776016a92155b4bac87d08b6a68263876ac38","object":"response","created_at":1787958619,"status":"in_progress","background":false,"completed_at":null,"error":null,"frequency_penalty":0.0,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"max_tool_calls":null,"model":"gpt-5.6-sol","moderation":null,"output":[],"parallel_tool_calls":true,"presence_penalty":0.0,"previous_response_id":null,"prompt_cache_key":"e8173a4f-6287-4be3-9e1d-0cbdf3484b31","prompt_cache_retention":"24h","reasoning":{"context":"all_turns","effort":"medium","mode":"standard","summary":null},"safety_identifier":"user-W8dEzFg1YVbssimY14LR1oK6","service_tier":"auto","store":false,"temperature":1.0,"text":{"format":{"type":"text"},"verbosity":"medium"},"tool_choice":"auto","tool_usage":{"image_gen":{"input_tokens":0,"input_tokens_details":{"image_tokens":0,"text_tokens":0},"output_tokens":0,"output_tokens_details":{"image_tokens":0,"text_tokens":0},"total_tokens":0},"web_search":{"num_requests":0}},"tools":[],"top_logprobs":0,"top_p":0.98,"truncation":"disabled","usage":null,"user":null,"metadata":{}},"sequence_number":0}

event: response.output_text.delta
data: {"type":"response.output_text.delta","content_index":0,"delta":"你好","item_id":"msg_04cbc44a7286b776016a92155c2f3487d0a5a53f71c1c65134","logprobs":[],"obfuscation":"ZQSLoVPPjaJfgF","output_index":0,"sequence_number":4}

event: response.output_text.delta
data: {"type":"response.output_text.delta","content_index":0,"delta":"！","item_id":"msg_04cbc44a7286b776016a92155c2f3487d0a5a53f71c1c65134","logprobs":[],"obfuscation":"08D3kV1kydgTn7N","output_index":0,"sequence_number":5}
"""

parsed_text = ""
for line in accumulated_raw.split("\n"):
    if line.startswith("data: ") and line != "data: [DONE]":
        json_str = line[6:]
        try:
            parsed = json.loads(json_str)
            if "choices" in parsed:
                parsed_text += parsed["choices"][0]["delta"].get("content", "")
            elif "type" in parsed:
                if parsed["type"] in ["response.text.delta", "response.output_text.delta"]:
                    parsed_text += parsed.get("delta", "")
        except:
            pass

print("Parsed:", parsed_text)
