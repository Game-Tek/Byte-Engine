# Documentation
Always write struct and trait documentation as:
The `<NAME>` struct/trait <PURPOSE>

If functions is larger than a few lines, write a short description of what the function does.

# Errors
When writing errors, first write a succint error message and then a sentence with the most likely cause of the error.

# API Design
When designing APIs, prefer composition over inheritance. Use traits to define shared behavior and structs to encapsulate data.
Prefer pure functions. Just inputs and outputs.
