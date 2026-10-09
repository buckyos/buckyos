<group_chat time="{{ input.time }}">
{% for msg in input.messages %}
{{ msg | render_format: "message.xml" }}
{% endfor %}
</group_chat>
Reply only if you were mentioned or the message is clearly addressed to you.
