require_relative '../spec_helper'

describe "oxen stash" do
  include_context "oxen test_repo"

  before do
    # Setup a test repo
    init_oxen_repo
    @test_file = "test_file.txt"
    File.write(@test_file, "initial content")
    run_oxen_cmd("add #{@test_file}")
    run_oxen_cmd("commit -m \"Initial commit\"")
  end

  it "should stash modified file and pop it" do
    # Modify the file
    File.write(@test_file, "modified content")

    # Stash the changes
    output = run_oxen_cmd("stash push -m \"my test stash\"")
    expect(output).to include("Created stash: stash_") # Check for timestamped stash name

    # File should be reverted to initial content
    expect(File.read(@test_file)).to eq("initial content")

    # Pop the stash
    output = run_oxen_cmd("stash pop")
    expect(output).to include("Popping stash: stash_")
    expect(output).to include("my test stash") # Check if message is displayed

    # File should now have modified content
    expect(File.read(@test_file)).to eq("modified content")

    # Stash list should be empty
    output = run_oxen_cmd("stash list")
    expect(output).to include("No stashes available.")
  end

  it "should stash a new file and pop it" do
    new_file = "new_tracking_file.txt"
    File.write(new_file, "new file content")

    # Stash the new file
    # For new files, they are not in HEAD, so 'stash push' effectively 'removes' them from WD
    # by not restoring them from a (non-existent) HEAD version.
    output = run_oxen_cmd("stash push -m \"stashing new file\"")
    expect(output).to include("Created stash: stash_")
    expect(File.exist?(new_file)).to be_falsey # New file should be removed by push logic

    # Pop the stash
    output = run_oxen_cmd("stash pop")
    expect(output).to include("Popping stash: stash_")
    expect(output).to include("stashing new file")

    # New file should be restored
    expect(File.exist?(new_file)).to be_truthy
    expect(File.read(new_file)).to eq("new file content")
  end

  it "should apply a stash and keep it" do
    File.write(@test_file, "content for apply")
    run_oxen_cmd("stash push -m \"apply-test\"")
    expect(File.read(@test_file)).to eq("initial content") # Reverted

    # Apply the stash
    output = run_oxen_cmd("stash apply")
    expect(output).to include("Applying stash: stash_")
    expect(output).to include("apply-test")
    expect(File.read(@test_file)).to eq("content for apply") # Applied

    # Stash should still exist
    output = run_oxen_cmd("stash list")
    expect(output).to include("stash_")
    expect(output).to include("apply-test")

    # Clean up by popping
    run_oxen_cmd("stash pop")
  end

  it "should list multiple stashes" do
    File.write(@test_file, "change 1")
    run_oxen_cmd("stash push -m \"first stash\"")

    File.write(@test_file, "change 2")
    run_oxen_cmd("stash push -m \"second stash\"")

    output = run_oxen_cmd("stash list")
    expect(output).to include("Available stashes:")
    expect(output).to include("first stash")
    expect(output).to include("second stash")
    stashes = output.scan(/stash_\d+/)
    expect(stashes.size).to eq(2)

    # Clean up
    run_oxen_cmd("stash pop")
    run_oxen_cmd("stash pop")
  end

  it "should stash push without a message" do
    File.write(@test_file, "no message content")
    output = run_oxen_cmd("stash push")
    expect(output).to include("Created stash: stash_")
    expect(File.read(@test_file)).to eq("initial content")

    output = run_oxen_cmd("stash list")
    # Check for stash name, but not a specific message (as none was given)
    # The format is ' - stash_timestamp' if no message
    expect(output).to match(/ - stash_\d+$/)


    output = run_oxen_cmd("stash pop")
    expect(output).to include("Popping stash: stash_")
    # Expect no specific message part in the pop output, e.g. "stash_timestamp - "
    expect(output).not_to include(" - ") if output.include?("Popping stash: stash_") && !output.match(/stash_\d+ - /)


    expect(File.read(@test_file)).to eq("no message content")
  end

  it "should do nothing if no changes to stash on push" do
    output = run_oxen_cmd("stash push")
    expect(output).to include("No changes to stash.")
  end

  it "should report no stashes on pop, apply, or list when empty" do
    output = run_oxen_cmd("stash pop")
    expect(output).to include("No stashes to pop.")

    output = run_oxen_cmd("stash apply")
    expect(output).to include("No stashes to apply.")

    output = run_oxen_cmd("stash list")
    expect(output).to include("No stashes available.")
  end
end
